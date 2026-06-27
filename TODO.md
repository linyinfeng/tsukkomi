# TODO — Comprehensive Code Review

> Generated from full-project review on 2026-06-27.
> Status: **18 unit tests pass, clippy clean, but multiple design/robustness gaps identified.**

---

## Error-Handling Philosophy

| Error Source | Strategy |
|-------------|----------|
| **User output** (LLM bad JSON, image download fails, malformed user message) | Gracefully handle, log, and degrade (e.g., retry, skip, return `Ok(None)`) |
| **Admin / config errors** (missing prompt file, bad API key, corrupted data files) | **Let it crash** — fail fast with a clear message so the operator notices |
| **Program logic invariants** (unauthenticated client past login, impossible state) | `unwrap()` or `expect()` with a descriptive message; these are bugs |

> *Non-user-output errors should never be silently swallowed.*

---

## Legend

| Priority | Meaning |
|----------|---------|
| **P0** | Data loss, crash, or security risk — fix before production |
| **P1** | Reliability / correctness issue — fix before next release |
| **P2** | Maintainability / tech debt — fix when touching related code |
| **P3** | Polish / optimization — nice to have |

---

## 1. Concurrency & Async Safety

### 1.1 [P0] `std::sync::Mutex` used inside async runtime
- **Where:** `crates/tsukkomi/src/chat.rs:252–305`
- **What:** `last_reply: Mutex<HashMap<String, Instant>>` is `std::sync::Mutex`. Calling `.lock().unwrap()` in async `reply()` and `reply_inner()` blocks the Tokio worker thread.
- **Impact:** Under load, worker threads stall; debounce check becomes a latency bottleneck; potential for priority inversion.
- **Plan:** Replace with `tokio::sync::Mutex` (or `parking_lot::Mutex` if sync semantics are truly needed). Since the critical section is tiny, `tokio::sync::Mutex` is the idiomatic choice.

### 1.2 [P1] `CURRENT_ROOM` task-local may leak across spawned tasks
- **Where:** `crates/tsukkomi/src/chat.rs:270–274`
- **What:** `CURRENT_ROOM.scope()` sets the task-local for the current async task. If `reply_inner` internally spawns additional tasks (e.g., via `tokio::spawn`) without propagating the scope, `CURRENT_ROOM.try_with()` in `Remember::call` / `Forget::call` will fail.
- **Impact:** Tool calls (`remember`/`forget`) sporadically returning "No room context available".
- **Plan:** Audit all internal async boundaries. Ensure any spawned sub-tasks either inherit the task-local or pass `room_id` explicitly. Consider refactoring `Tool` impls to accept `room_id` as an explicit parameter instead of relying on task-local state.

---

## 2. Data Safety & Persistence

### 2.1 [P0] Non-atomic file writes risk data loss on crash
- **Where:**
  - `crates/tsukkomi/src/memory/store.rs:116–118` (`modify`)
  - `crates/tsukkomi/src/memory/file.rs:55–62` (`replace_all`)
- **What:** Both functions truncate the file (`set_len(0)`) before writing new content. A crash or power loss between truncation and write leaves an empty or partially-written file.
- **Impact:** Total loss of a room's conversation history or long-term memories.
- **Plan:** Implement atomic writes: write to a temp file in the same directory, then `fs::rename` to overwrite the target. Use a helper like `atomic_write(path, bytes) -> io::Result<()>` shared by both modules.

### 2.2 [P1] Path traversal in file-based memory
- **Where:**
  - `crates/tsukkomi/src/memory/store.rs:41–42`
  - `crates/tsukkomi/src/memory/file.rs:22–23`
- **What:** `conversation_id` / `room_id` is interpolated directly into a filename with no sanitization. A malicious or accidental room ID like `../../../etc/passwd` writes outside the base directory.
- **Impact:** Arbitrary file overwrite (though limited by OS permissions).
- **Plan:** Sanitize IDs before using as filenames. Options:
  - URL-safe Base64 encode the raw ID.
  - Replace path separators with safe characters and validate against traversal patterns.
  - Maintain a mapping file from logical IDs to safe filenames.

### 2.3 [P1] Corrupted `.jsonl` should fail fast, not be silently skipped
- **Where:** `crates/tsukkomi/src/memory/file.rs:96–99`
- **What:** `load()` iterates lines and calls `serde_json::from_str(line)?`. One malformed line causes the entire `load` to fail.
- **Impact:** A single bad line (e.g., partial write, disk corruption) renders the entire conversation history unreadable.
- **Rationale:** This is an **administrator / data-integrity error** (non-user output), so it should **let it crash**. Silently skipping lines masks data loss.
- **Plan:**
  - Keep the current fail-fast behavior (do NOT silently skip lines).
  - Improve the error message to include the line number and a snippet of the bad line for easier debugging.
  - Add a `.jsonl.bak` mechanism on successful `replace_all` so admins can recover from the backup if the main file is corrupted.
  - The real fix is **atomic writes** (see 2.1) — prevents the corruption from happening in the first place.

### 2.4 [P1] No message deduplication (Matrix)
- **Where:** `crates/tsukkomi-matrix/src/main.rs:221–227`
- **What:** Messages are skipped only by comparing `origin_server_ts` to startup time. After a sync restart or reconnection, already-processed messages with timestamps after startup can be re-fed to the LLM.
- **Impact:** Bot may reply twice to the same message; conversation state becomes inconsistent.
- **Plan:** Track processed event IDs in a small on-disk set (e.g., SQLite or Bloom filter) per room. Skip any event whose ID has been seen. Expire entries older than N days.

### 2.5 [P2] No startup-time filtering (Telegram)
- **Where:** `crates/tsukkomi-telegram/src/main.rs`
- **What:** Unlike Matrix, Telegram handler has no `StartupTime` filter. On first poll or after a long outage, old messages may be processed.
- **Impact:** Bot may reply to hours-old messages when restarted.
- **Plan:** Add a `StartupTime` context and skip messages with `msg.date < startup`, matching the Matrix behavior.

---

## 3. Error Handling & Robustness

### 3.1 [P2] `panic!` in `system_prompt` on missing prompt file — acceptable for admin errors
- **Where:** `crates/tsukkomi/src/chat.rs:188–189`
- **What:** `std::fs::read_to_string(path).unwrap_or_else(|e| panic!(...))` aborts the process if the custom system prompt file is unreadable.
- **Impact:** Bot crashes at startup for a configuration typo.
- **Rationale:** This is an **administrator configuration error** (non-user output), so **let it crash** is the correct behavior. A missing prompt file means the deployment is misconfigured.
- **Plan:** Improve the panic message to be actionable (e.g., include the exact path and suggest checking the env var or CLI flag). Keep the panic — do NOT change to `Result`.

### 3.2 [P2] `unwrap()` on `client.user_id()` in Matrix main — acceptable for logic errors
- **Where:** `crates/tsukkomi-matrix/src/main.rs:84`
- **What:** `client.user_id().unwrap()` panics if the client is not authenticated.
- **Impact:** Crash if session handling has a bug.
- **Rationale:** Reaching this point without authentication is a **program logic error** (not user output). The preceding `ensure_session` should have already errored out. If we get here unauthenticated, something is fundamentally wrong.
- **Plan:** Add a comment explaining why this invariant must hold. Keep the `unwrap()` — changing to `Result` just defers a crash that should never happen.

### 3.3 [P2] Session file parse failure silently re-logins without warning
- **Where:** `crates/tsukkomi-matrix/src/main.rs:112–122`
- **What:** Any failure reading or parsing the session file falls through to `do_login`. The user is not informed that their saved session was discarded.
- **Impact:** Unexpected new device creation, potentially invalidating the previous session on the homeserver.
- **Plan:** Log a `warn!` before falling back to login, including the specific error (file not found vs. parse failure vs. restore failure).

### 3.4 [P2] `unwrap_or_default()` on invalid Matrix timestamp
- **Where:** `crates/tsukkomi-matrix/src/main.rs:223–224`
- **What:** `from_timestamp_millis(...).unwrap_or_default()` returns epoch (1970-01-01) for invalid timestamps. Since `sent_at < startup` is the guard, epoch is always < startup, so the message is silently skipped.
- **Impact:** Legitimate messages with edge-case timestamps could be dropped without any log.
- **Plan:** Log a `warn!` when timestamp conversion fails, and skip the message explicitly.

### 3.5 [P2] `max_retries` exhaustion returns `Ok(None)` instead of error
- **Where:** `crates/tsukkomi/src/chat.rs:317–318`
- **What:** After all retries fail, the function returns `Ok(None)` rather than an error.
- **Impact:** Callers (Matrix/Telegram) cannot distinguish "AI chose not to reply" from "LLM is down / misbehaving". No metric or alert can be triggered.
- **Plan:** Return `Err(anyhow!("all {max_retries} retries exhausted"))` so the caller can log/error appropriately.

### 3.6 [P2] Matrix recovery key failure is silently swallowed
- **Where:** `crates/tsukkomi-matrix/src/main.rs:168–178`
- **What:** `try_import_recovery_key` logs a warning on failure but never propagates the error.
- **Impact:** Operator may not notice that recovery key import failed, leading to undecryptable messages.
- **Plan:** Consider returning `anyhow::Result<()>` and letting the caller decide whether recovery-key failure is fatal. At minimum, log at `error!` level.

---

## 4. Security

### 4.1 [P1] Matrix password logged in debug output
- **Where:** `crates/tsukkomi-matrix/src/main.rs:58–62`
- **What:** `tracing::debug!(matrix_recovery_key = opts.matrix_recovery_key.is_some(), "Parsed options");` is safe, but note that `Options` derives `Debug` and is logged at line 36: `tracing::debug!(?opts, "Parsed options")` (in Telegram). If Matrix adds similar debug logging, the password could leak.
- **Impact:** Credential exposure in logs.
- **Plan:**
  - Add `#[arg(hide = true)]` to `password` in Matrix `Options` (already present for `matrix_recovery_key`).
  - Never `#[derive(Debug)]` or log structs containing secrets; implement a custom `Debug` that redacts sensitive fields.

### 4.2 [P2] NixOS module lacks systemd sandboxing
- **Where:** `nixos/tsukkomi.nix`
- **What:** The systemd services run with default privileges. No `PrivateTmp`, `ProtectSystem`, `NoNewPrivileges`, etc.
- **Impact:** If the bot is compromised, the attacker has broad system access.
- **Plan:** Add standard hardening options:
  ```nix
  PrivateTmp = true;
  ProtectSystem = "strict";
  ProtectHome = true;
  NoNewPrivileges = true;
  RestrictSUIDSGID = true;
  ```
  Bind-read only paths for credentials, read-write for `StateDirectory`.

### 4.3 [P2] Shared `StateDirectory` between Matrix and Telegram services
- **Where:** `nixos/tsukkomi.nix:145–146`, `188–189`
- **What:** Both systemd services use `StateDirectory = "tsukkomi"` and `WorkingDirectory = "/var/lib/tsukkomi"`. If both backends are enabled on the same host, their relative-path memory directories collide.
- **Impact:** Cross-platform memory pollution, potential information leak between backends.
- **Plan:** Use separate state directories (`tsukkomi-matrix` and `tsukkomi-telegram`), or ensure each binary defaults to a backend-specific subdirectory (e.g., `memory/matrix/` and `memory/telegram/`).

---

## 5. Code Quality & Maintainability

### 5.1 [P2] `system_prompt` regenerates JSON schemas on every call
- **Where:** `crates/tsukkomi/src/chat.rs:86–98`
- **What:** `format_system_prompt()` calls `schema_for!()` and `serde_json::to_string_pretty()` every time `system_prompt()` is invoked. These are compile-time schemas; the result never changes at runtime.
- **Impact:** Unnecessary CPU and allocation overhead on every `ChatManager` construction.
- **Plan:** Cache the formatted schema strings in `once_cell::sync::Lazy<String>` or `std::sync::OnceLock`.

### 5.2 [P2] `format_system_prompt()` has hardcoded Chinese/English mix
- **Where:** `crates/tsukkomi/src/chat.rs:92–97`
- **What:** The formatting string mixes Chinese and English instructions inline. This is fine functionally but makes localization or prompt tuning harder.
- **Impact:** Minor — code readability and maintainability.
- **Plan:** Extract the wrapper template into a dedicated `.md` file (e.g., `prompts/schemas.md`) and use `include_str!`, consistent with other prompts.

### 5.3 [P2] `tsukkomi-telegram/main.rs:82` `_opts` unused
- **Where:** `crates/tsukkomi-telegram/src/main.rs:82`
- **What:** `Arc<Options>` is injected via dependency tree but never read inside `msg_handler`.
- **Impact:** Dead code / confusing to readers.
- **Plan:** Remove `_opts` from `msg_handler` signature and from the `Dispatcher` dependencies if it's not needed. If it's reserved for future filtering, add a `// TODO: use opts for ...` comment.

### 5.4 [P2] `ChatManager` generic parameters are never used outside default type
- **Where:** `crates/tsukkomi/src/chat.rs:104–180`
- **What:** `ChatManager<M, I>` is generic over two completion models, but `new()` is only implemented for `ChatManager<DeepSeekModel, MiMoModel>`. No other constructors or usages exist.
- **Impact:** Unnecessary generic complexity; `ChatManager` could be a concrete type.
- **Plan:** Either:
  - Provide a generic `new()` that accepts any `CompletionModel`, or
  - Remove the generics and make `ChatManager` a concrete struct, simplifying the code.

### 5.5 [P2] `CompactingMemory` vs manual compaction is confusing
- **Where:** `crates/tsukkomi/src/chat.rs:321–375`
- **What:** `compact_before_prompt` returns `Vec<Message>` but the return value is discarded (`let _messages = ...`). The comment explains *why* manual compaction is needed, but the discarding is surprising.
- **Impact:** New contributors may think the compaction result should be used.
- **Plan:** Return `Result<(), _>` instead of `Vec<Message>` to make it clear the function is side-effect only. Alternatively, rename to `ensure_compacted(room_id)`.

### 5.6 [P2] Default `batch_size` (200) equals `sliding_window` (200)
- **Where:** `crates/tsukkomi/src/cli.rs:29–30`
- **What:** `batch_size` default is `200`, same as `sliding_window`. Demotion only happens when messages exceed `window_size + batch_size` (400). With 200 messages, compaction never triggers even though the window is full.
- **Impact:** Defaults may surprise users who expect compaction at 200 messages.
- **Plan:** Align the default with the documented value in `AGENTS.md` (100) or explicitly document that compaction triggers at `window + batch`.

### 5.7 [P3] `utils.rs::init_tracing()` panics on double-init
- **Where:** `crates/tsukkomi/src/utils.rs:1–5`
- **What:** `tracing_subscriber::fmt().init()` panics if called twice.
- **Impact:** Tests that call `init_tracing` would panic, though currently no tests do.
- **Plan:** Use `tracing_subscriber::fmt().try_init().ok()` to make it idempotent.

---

## 6. Missing Features / Behavioral Gaps

### 6.1 [P1] Matrix bot does not track `reply_to_user_id`
- **Where:** `crates/tsukkomi-matrix/src/main.rs:229–274`
- **What:** `ChatInput.reply_to_user_id` is always `None` for Matrix messages. The prompt instructs the LLM to use this field heavily for conversation awareness.
- **Impact:** Bot cannot tell when it is being directly replied to vs. observing a conversation between others.
- **Plan:** Parse the `m.relates_to` field in `OriginalSyncRoomMessageEvent`. If `relates_to.in_reply_to.event_id` resolves to a message sent by the bot, set `reply_to_user_id` to the bot's own ID.

### 6.2 [P1] No graceful shutdown for Matrix bot
- **Where:** `crates/tsukkomi-matrix/src/main.rs:103–108`
- **What:** Infinite `client.sync()` loop with no signal handling. `Ctrl+C` kills the process abruptly.
- **Impact:** Potential for in-flight memory writes to be truncated (compounding the non-atomic write issue).
- **Plan:**
  - Add `tokio::signal::ctrl_c()` listener.
  - Use `client.sync_with_callback(SyncSettings::default(), |response| { ... })` and check a shutdown `tokio::sync::watch` channel in the callback to stop syncing gracefully.

### 6.3 [P2] Image caption extraction in Matrix is heuristic-based
- **Where:** `crates/tsukkomi-matrix/src/main.rs:256–260`
- **What:** Caption is inferred by comparing `image.body` to `image.filename`. This is a convention, not a reliable API guarantee.
- **Impact:** Image captions may be silently dropped or incorrectly treated as text.
- **Plan:** Research matrix-rust-sdk's `RoomMessageEventContent` structure. Prefer explicit `caption` / `filename` fields if available in newer SDK versions. At minimum, log when the heuristic is applied.

### 6.4 [P2] No rate limiting or backpressure on LLM calls
- **Where:** `crates/tsukkomi/src/chat.rs:250–319`
- **What:** Every incoming message triggers an immediate LLM prompt. In a busy room, this can exhaust API quotas or hit rate limits.
- **Impact:** API costs spike; provider may throttle or ban the key.
- **Plan:** Add a per-room semaphore or token-bucket rate limiter (e.g., `tokio::sync::Semaphore` or ` governor` crate). Reject or queue messages that exceed the limit.

### 6.5 [P2] No health check or liveness probe endpoint
- **Where:** Both binaries
- **What:** No HTTP endpoint or mechanism to verify the bot is alive and connected.
- **Impact:** Hard to monitor in production (Kubernetes, systemd watchdog, etc.).
- **Plan:** Add an optional `/healthz` HTTP endpoint (using `axum` or `hyper`) that returns 200 when the bot's event loop is running. Alternatively, implement `sd_notify` for systemd.

---

## 7. Dependency & Build Issues

### 7.1 [P1] Workspace dependencies use unbounded `*` versions
- **Where:** `Cargo.toml` workspace root
- **What:** `tokio = "*"`, `rig = "*"`, `anyhow = "*"`, etc. No version constraints.
- **Impact:** Future `cargo update` can pull in breaking changes without warning. `rig` is especially volatile as a young crate.
- **Plan:** Pin to current minor versions (e.g., `tokio = "1"`, `anyhow = "1"`, `rig = "0.11"`). Use `cargo update` deliberately, not accidentally.

### 7.2 [P2] `cargo nextest` not available in devShell
- **Where:** `flake.nix:116–122`
- **What:** `checks.nextest` uses `craneLib.cargoNextest` in CI, but `devShells.default` does not include `cargo-nextest`.
- **Impact:** Developers cannot run the same test command locally as CI (`cargo nextest run`).
- **Plan:** Add `cargo-nextest` to `devShells.default.packages`.

### 7.3 [P2] Rust toolchain not pinned in devShell
- **Where:** `flake.nix:119`
- **What:** `rustup` is provided, but no specific toolchain version is enforced. Edition 2024 requires Rust >= 1.85.
- **Impact:** Developers with an old default toolchain get cryptic compile errors about unsupported edition.
- **Plan:** Use `fenix` or `oxalica/rust-overlay` in the flake to pin a specific Rust version (e.g., `rustc 1.85.0`), or add a `rust-toolchain.toml` file.

### 7.4 [P2] Inconsistent dependency declaration pattern
- **Where:** Various `Cargo.toml` files
- **What:** Some deps are in workspace root (`anyhow.workspace = true`), others are crate-local with `"*"` (`async-fd-lock = "*"`, `humantime = "*"`, `tempfile = "*"`).
- **Impact:** Hard to audit versions; risk of version drift.
- **Plan:** Move all shared dependencies to `[workspace.dependencies]` and use `workspace = true` everywhere. Keep only crate-specific deps (e.g., `matrix-sdk`) local.

---

## 8. Testing Gaps

### 8.1 [P1] No tests for `compactor.rs`
- **Where:** `crates/tsukkomi/src/compactor.rs`
- **What:** Zero test coverage for the compaction logic, which is critical for data integrity.
- **Plan:** Add unit tests that mock the `Agent` (or use a test double) and verify:
  - Compacted output is a valid `Message::System`.
  - `carry_over` is prepended correctly.
  - Errors are mapped to `MemoryError::Backend`.

### 8.2 [P1] No tests for `file.rs`
- **Where:** `crates/tsukkomi/src/memory/file.rs`
- **What:** File I/O operations (load, append, replace_all, clear) are untested.
- **Plan:** Use `tempfile::TempDir` to create temporary directories. Test:
  - Round-trip: append → load returns same messages.
  - `replace_all` overwrites previous content.
  - `clear` removes the file.
  - Graceful handling of missing files.

### 8.3 [P2] No integration tests for `ChatManager::reply`
- **Where:** `crates/tsukkomi/src/chat.rs`
- **What:** The orchestration logic (debounce, image description, retry loop, compaction trigger) is untested.
- **Plan:** Add a test module with a mock `CompletionModel` that returns predetermined responses. Verify:
  - Debounce prevents rapid re-reply.
  - Retry loop attempts up to `max_retries`.
  - `Skip` payload returns `Ok(None)`.
  - Compaction is triggered when message count exceeds threshold.

### 8.4 [P2] Binary crates have zero tests
- **Where:** `tsukkomi-matrix`, `tsukkomi-telegram`
- **What:** No unit or integration tests for either bot binary.
- **Plan:** Add at least smoke tests for CLI parsing (e.g., `Options::try_parse_from`) and event handler logic where feasible.

---

## 9. Documentation & Ops

### 9.1 [P2] No README
- **Where:** Project root
- **What:** No `README.md` explaining what the project is, how to build it, or how to run it.
- **Plan:** Add a `README.md` with:
  - Project description and architecture overview.
  - Quick start (Nix flake, `cargo run`).
  - Required environment variables.
  - Link to `AGENTS.md` for developer conventions.

### 9.2 [P2] NixOS module `telegram.chats` type mismatch risk
- **Where:** `nixos/tsukkomi.nix:91–95`
- **What:** `telegram.chats` is `listOf str`, but the CLI expects `Vec<i64>`. The module concatenates strings and passes them to `--chats`, relying on clap to parse.
- **Impact:** A non-numeric chat ID in the Nix config causes a runtime parse error at bot startup.
- **Plan:** Change the option type to `listOf int` (or add an assertion that each element parses as `i64`).

### 9.3 [P3] `AGENTS.md` conventions partially not enforced
- **Where:** `.opencode/AGENTS.md` (project conventions)
- **What:** Conventions mention `cargo nextest run` and `cargo clippy`, but CI is Nix-based (`nix flake check`). New contributors may not realize `nix fmt` is required.
- **Plan:** Add a `CONTRIBUTING.md` that explicitly lists the pre-commit checklist and links to the Nix setup.

---

## Summary Table

| # | Issue | Priority | File(s) | Category |
|---|-------|----------|---------|----------|
| 1 | `std::sync::Mutex` in async | P0 | `chat.rs` | Concurrency |
| 2 | Non-atomic file writes | P0 | `store.rs`, `file.rs` | Data Safety |
| 3 | Path traversal in filenames | P1 | `store.rs`, `file.rs` | Security |
| 4 | No message deduplication (Matrix) | P1 | `tsukkomi-matrix/main.rs` | Reliability |
| 5 | `panic!` on missing prompt file | P1 | `chat.rs` | Error Handling |
| 6 | `unwrap()` on `client.user_id()` | P1 | `tsukkomi-matrix/main.rs` | Error Handling |
| 7 | `CURRENT_ROOM` task-local leakage | P1 | `chat.rs`, `store.rs` | Concurrency |
| 8 | Corrupted `.jsonl` kills entire load | P1 | `file.rs` | Data Safety |
| 9 | No startup-time filter (Telegram) | P2 | `tsukkomi-telegram/main.rs` | Reliability |
| 10 | Session parse failure silent | P2 | `tsukkomi-matrix/main.rs` | Error Handling |
| 11 | `max_retries` returns `Ok(None)` | P2 | `chat.rs` | Error Handling |
| 12 | Password potentially loggable | P2 | `tsukkomi-matrix/main.rs` | Security |
| 13 | NixOS module lacks sandboxing | P2 | `nixos/tsukkomi.nix` | Security |
| 14 | Shared `StateDirectory` collision | P2 | `nixos/tsukkomi.nix` | Security |
| 15 | `system_prompt` regenerates schemas | P2 | `chat.rs` | Performance |
| 16 | Unused `_opts` in Telegram handler | P2 | `tsukkomi-telegram/main.rs` | Cleanup |
| 17 | `ChatManager` unnecessary generics | P2 | `chat.rs` | Design |
| 18 | Default `batch_size` equals window | P2 | `cli.rs` | Config |
| 19 | Matrix lacks `reply_to_user_id` | P1 | `tsukkomi-matrix/main.rs` | Feature Gap |
| 20 | No graceful shutdown (Matrix) | P1 | `tsukkomi-matrix/main.rs` | Feature Gap |
| 21 | No rate limiting on LLM calls | P2 | `chat.rs` | Feature Gap |
| 22 | No health check endpoint | P2 | Both binaries | Observability |
| 23 | Unbounded `*` dependencies | P1 | `Cargo.toml` | Build |
| 24 | `cargo nextest` missing in devShell | P2 | `flake.nix` | Build |
| 25 | Rust toolchain not pinned | P2 | `flake.nix` | Build |
| 26 | Inconsistent dep declarations | P2 | Various `Cargo.toml` | Build |
| 27 | No tests for `compactor.rs` | P1 | `compactor.rs` | Testing |
| 28 | No tests for `file.rs` | P1 | `file.rs` | Testing |
| 29 | No integration tests for `reply` | P2 | `chat.rs` | Testing |
| 30 | Binary crates have zero tests | P2 | Both binaries | Testing |
| 31 | No README | P2 | Root | Documentation |
| 32 | NixOS `telegram.chats` type risk | P2 | `nixos/tsukkomi.nix` | Config |
| 33 | `init_tracing` panics on double-init | P3 | `utils.rs` | Robustness |
| 34 | Image caption heuristic | P2 | `tsukkomi-matrix/main.rs` | Robustness |
| 35 | `compact_before_prompt` return value | P2 | `chat.rs` | Clarity |

---

## Recommended Execution Order

1. **Fix data safety first** (atomic writes, path traversal, `.jsonl` corruption handling).
2. **Fix async safety** (`std::sync::Mutex` → `tokio::sync::Mutex`).
3. **Fix error handling** (remove `panic!` and `unwrap()` in production paths).
4. **Add critical missing features** (Matrix reply tracking, graceful shutdown, Telegram startup filter).
5. **Harden NixOS module** (sandboxing, separate state dirs).
6. **Pin dependencies** and fix build ergonomics (`nextest`, Rust toolchain).
7. **Backfill tests** for `file.rs`, `compactor.rs`, and `ChatManager` logic.
8. **Polish** (README, dead code removal, performance caching).
