# TODO — Comprehensive Code Review

> Updated by full-project rescan on 2026-06-28.
> Status: **46 tests pass, clippy clean. Several fixes applied since initial review; remaining gaps documented below.**

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
- **Where:** `crates/tsukkomi/src/chat.rs:3,113,176,248-251,299-302`
- **What:** `last_reply: Mutex<HashMap<String, Instant>>` is `std::sync::Mutex`. Calling `.lock().unwrap()` in async `reply()` and `reply_inner()` blocks the Tokio worker thread.
- **Impact:** Under load, worker threads stall; debounce check becomes a latency bottleneck.
- **Plan:** Replace with `tokio::sync::Mutex`. The lock is never held across `.await` so no deadlock risk, but blocking the worker thread is still wrong.
- **Note:** Memory lock registries (`store.rs`, `file.rs`) were already fixed to `tokio::sync::Mutex` (commit `7245df4`); `chat.rs` was missed.

### 1.2 [P1] `CURRENT_ROOM` task-local may leak across spawned tasks
- **Where:** `crates/tsukkomi/src/memory/store.rs:14-16`, `crates/tsukkomi/src/chat.rs:266-269`
- **What:** `CURRENT_ROOM.scope()` only applies within the current task. If `rig`'s agent internally spawns tasks, `Remember::call` / `Forget::call` using `CURRENT_ROOM.try_with()` will fail with `StoreError::NoRoomContext`.
- **Plan:** Audit all internal async boundaries. Refactor `Tool` impls to accept `room_id` as an explicit parameter instead of relying on task-local state.

---

## 2. Data Safety & Persistence

### 2.1 [P1] Path traversal in file-based memory
- **Where:** `crates/tsukkomi/src/memory/store.rs:55`, `crates/tsukkomi/src/memory/file.rs:36`
- **What:** `room_id` / `conversation_id` is interpolated directly into a filename with no sanitization (`format!("{conversation_id}.jsonl")`). A malicious or accidental room ID like `../../../etc/passwd` writes outside the base directory.
- **Plan:** Sanitize IDs before using as filenames — URL-safe Base64 encode the raw ID, or replace path separators and validate against traversal patterns.

### 2.2 [P1] Corrupted `.jsonl` should fail fast with better diagnostics
- **Where:** `crates/tsukkomi/src/memory/file.rs:90-93`
- **What:** `load()` iterates lines and calls `serde_json::from_str(line)?`. One malformed line causes the entire `load` to fail.
- **Rationale:** This is an **administrator / data-integrity error** — fail-fast is correct. But the error message should include the line number and a snippet of the bad line.
- **Plan:**
  - Keep fail-fast behavior.
  - Improve the error to include line number and bad-line snippet.
  - Add a `.jsonl.bak` mechanism on successful `replace_all` so admins can recover.
  - Atomic writes (`memory/utils.rs`) already prevent partial-write corruption.

### 2.3 [P1] No message deduplication (Matrix)
- **Where:** `crates/tsukkomi-matrix/src/main.rs:221-235`
- **What:** Messages are skipped only by comparing `origin_server_ts` to startup time. After a sync restart or reconnection, already-processed messages can be re-fed to the LLM.
- **Plan:** Track processed event IDs in a small on-disk set (e.g., SQLite or Bloom filter) per room. Expire entries older than N days.

### 2.4 [P2] No startup-time filtering (Telegram)
- **Where:** `crates/tsukkomi-telegram/src/main.rs:85-144`
- **What:** Unlike Matrix, Telegram handler has no `StartupTime` filter. On first poll or after a long outage, old messages may be processed.
- **Plan:** Add a `StartupTime` context and skip messages with `msg.date < startup`, matching the Matrix behavior at `tsukkomi-matrix/src/main.rs:233-235`.

---

## 3. Error Handling & Robustness

### 3.1 [P2] `panic!` in `system_prompt` on missing prompt file — acceptable, improve message
- **Where:** `crates/tsukkomi/src/chat.rs:185`
- **What:** `panic!("Failed to read system prompt file {path}: {e}")` aborts the process if the custom system prompt file is unreadable.
- **Rationale:** Admin configuration error — **let it crash** is correct.
- **Plan:** Improve the panic message to be actionable (include exact path, suggest checking the env var / CLI flag). Keep the panic — do NOT change to `Result`.

### 3.2 [P2] `unwrap()` on `client.user_id()` in Matrix main — acceptable, add comment
- **Where:** `crates/tsukkomi-matrix/src/main.rs:88`
- **What:** `client.user_id().unwrap()` panics if the client is not authenticated.
- **Rationale:** Program logic invariant — `ensure_session` must have succeeded. If we get here unauthenticated, something is fundamentally wrong.
- **Plan:** Add a comment explaining why this invariant must hold. Keep the `unwrap()`.

### 3.3 [P2] Session file parse failure silently re-logins without warning
- **Where:** `crates/tsukkomi-matrix/src/main.rs:120-125`
- **What:** Both file-not-found and JSON-parse failure silently fall through to `do_login`. The operator is not informed that their saved session was discarded.
- **Plan:** Log a `warn!` before falling back to login, including the specific error (file not found vs. parse failure vs. restore failure).

### 3.4 [P2] `unwrap_or_default()` on invalid Matrix timestamp
- **Where:** `crates/tsukkomi-matrix/src/main.rs:231-232`
- **What:** `from_timestamp_millis(...).unwrap_or_default()` returns epoch (1970-01-01) for invalid timestamps. Since `sent_at < startup` is the guard, epoch passes the filter and the message is silently dropped without any log.
- **Plan:** Log a `warn!` when timestamp conversion fails, and skip the message explicitly.

### 3.5 [P2] `max_retries` exhaustion returns `Ok(None)` instead of error
- **Where:** `crates/tsukkomi/src/chat.rs:314-315`
- **What:** After all retries fail, the function returns `Ok(None)` rather than an error.
- **Impact:** Callers cannot distinguish "AI chose not to reply" from "LLM is down / misbehaving".
- **Plan:** Return `Err(anyhow!("all {max_retries} retries exhausted"))` so the caller can log appropriately. Check Matrix (`main.rs:284`) and Telegram (`main.rs:134`) call sites handle the `Err` variant.

---

## 4. Security

### 4.1 [P1] Telegram token leaked in debug log output
- **Where:** `crates/tsukkomi-telegram/src/main.rs:36`
- **What:** `tracing::debug!(?opts, "Parsed options")` logs the entire `Options` struct via `Debug`. The `token` field (`#[arg(long, env = "TELOXIDE_TOKEN")]` at line 13) is NOT hidden from `Debug` output — it appears in plaintext in debug logs.
- **Note:** The Matrix counterpart was fixed (now logs only `matrix_recovery_key.is_some()`), but Telegram still has this issue.
- **Plan:** Add `#[arg(hide = true)]` to the `token` field, or stop logging `?opts` entirely and log only non-sensitive fields.

### 4.2 [P2] NixOS module lacks systemd sandboxing
- **Where:** `nixos/tsukkomi.nix:142-156` (Matrix), `nixos/tsukkomi.nix:185-196` (Telegram)
- **What:** The systemd services run with default privileges. No `PrivateTmp`, `ProtectSystem`, `NoNewPrivileges`, etc.
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
- **Where:** `nixos/tsukkomi.nix:145,188`
- **What:** Both services use `StateDirectory = "tsukkomi"`. If both backends are enabled on one host, their memory files collide.
- **Plan:** Use separate state directories: `tsukkomi-matrix` and `tsukkomi-telegram`.

---

## 5. Code Quality & Maintainability

### 5.1 [P2] `system_prompt` regenerates JSON schemas on every call
- **Where:** `crates/tsukkomi/src/chat.rs:86-98`
- **What:** `format_system_prompt()` calls `schema_for!()` and `serde_json::to_string_pretty()` every time. In practice called only once from `ChatManager::new()`, so low severity.
- **Plan:** Cache the formatted schema strings in `once_cell::sync::Lazy<String>` or `std::sync::OnceLock`.

### 5.2 [P2] `format_system_prompt()` has hardcoded Chinese/English mix
- **Where:** `crates/tsukkomi/src/chat.rs:92-97`
- **What:** The formatting string mixes Chinese and English instructions inline. Makes prompt tuning harder.
- **Plan:** Extract the wrapper template into a dedicated `.md` file (e.g., `prompts/schemas.md`) and use `include_str!`, consistent with other prompts.

### 5.3 [P2] `ChatManager` generic parameters are never used outside default type
- **Where:** `crates/tsukkomi/src/chat.rs:106,117-180`
- **What:** `ChatManager<M, I>` is generic, but `new()` is only implemented for `ChatManager<DeepSeekModel, MiMoModel>`. No alternative constructors exist.
- **Plan:** Either provide a generic `new()` or remove the generics and make `ChatManager` a concrete struct.

### 5.4 [P2] `compact_before_prompt` returns `Vec<Message>` but is discarded
- **Where:** `crates/tsukkomi/src/chat.rs:278`
- **What:** `let _messages = self.compact_before_prompt(room_id).await;` — the return value is explicitly discarded. The side-effect (persisting to FileMemory) is the real purpose.
- **Plan:** Return `Result<(), _>` instead of `Vec<Message>`, or rename to `ensure_compacted(room_id)`.

### 5.5 [P2] Default `batch_size` (200) equals `sliding_window` (200)
- **Where:** `crates/tsukkomi/src/cli.rs:29-30`
- **What:** Demotion only happens when messages exceed `window_size + batch_size` (400). Users expecting compaction at 200 won't see it.
- **Plan:** Align the default with the documented value in `AGENTS.md` (100) or explicitly document that compaction triggers at `window + batch`.

### 5.6 [P3] `utils.rs::init_tracing()` panics on double-init
- **Where:** `crates/tsukkomi/src/utils.rs:4`
- **What:** `tracing_subscriber::fmt().init()` panics if called twice.
- **Plan:** Use `tracing_subscriber::fmt().try_init().ok()` to make it idempotent.

---

## 6. Missing Features / Behavioral Gaps

### 6.1 [P1] Matrix bot does not track `reply_to_user_id`
- **Where:** `crates/tsukkomi-matrix/src/main.rs:244,278`
- **What:** `ChatInput.reply_to_user_id` is always `None` for Matrix messages. The prompt instructs the LLM to use this field heavily. Contrast with Telegram which correctly extracts it (`tsukkomi-telegram/src/main.rs:96-98`).
- **Plan:** Parse `m.relates_to.in_reply_to.event_id` from `OriginalSyncRoomMessageEvent`. If the replied-to message was sent by the bot, set `reply_to_user_id` to the bot's own ID.

### 6.2 [P1] No graceful shutdown for Matrix bot
- **Where:** `crates/tsukkomi-matrix/src/main.rs:111-116`
- **What:** Infinite `client.sync()` loop with no signal handling. `Ctrl+C` kills the process abruptly.
- **Plan:**
  - Add `tokio::signal::ctrl_c()` listener.
  - Use `client.sync_with_callback()` and check a shutdown `tokio::sync::watch` channel in the callback.

### 6.3 [P2] Image caption extraction in Matrix is heuristic-based
- **Where:** `crates/tsukkomi-matrix/src/main.rs:264-268`
- **What:** Caption is inferred by comparing `image.body` to `image.filename`. This is a convention, not a reliable API guarantee.
- **Plan:** Research matrix-rust-sdk's `RoomMessageEventContent` for explicit `caption` / `filename` fields in newer SDK versions. At minimum, log when the heuristic is applied.

### 6.4 [P2] No rate limiting or backpressure on LLM calls
- **Where:** `crates/tsukkomi/src/chat.rs:289-293`
- **What:** Every incoming message triggers an immediate LLM prompt. In a busy room, this can exhaust API quotas.
- **Plan:** Add a per-room semaphore or token-bucket rate limiter (e.g., `tokio::sync::Semaphore` or `governor` crate).

### 6.5 [P2] No health check or liveness probe endpoint
- **Where:** Both binaries
- **What:** No HTTP endpoint or mechanism to verify the bot is alive and connected.
- **Plan:** Add an optional `/healthz` HTTP endpoint (using `axum` or `hyper`), or implement `sd_notify` for systemd.

### 6.6 [P2] Telegram stickers are not handled as images
- **Where:** `crates/tsukkomi-telegram/src/main.rs:130-131`
- **What:** Telegram stickers fall through to `else { return Ok(()) }` and are silently dropped. They are visually important message content the LLM should be aware of.
- **Plan:** Detect sticker messages, download the sticker image (WebP or TGS), convert if necessary, and emit a `MessageBody::Image` so the image-understanding agent can describe it.

### 6.7 [P2] Image understanding lacks caching
- **Where:** `crates/tsukkomi/src/chat.rs:206-243`
- **What:** `describe_images()` runs for every invocation with no deduplication. The same image in a reply thread or reposted triggers redundant LLM calls.
- **Plan:** Cache image descriptions by content hash (SHA-256 of downloaded bytes) or by URL. Store as in-memory LRU or on-disk alongside the memory store. Evict old entries to prevent unbounded growth.

---

## 7. Dependency & Build Issues

### 7.1 [P1] Workspace dependencies use unbounded `*` versions
- **Where:** `Cargo.toml:9-22` (all 13 workspace deps), `crates/tsukkomi/Cargo.toml:14,16,17,19,23` (5 crate-local)
- **What:** `tokio = "*"`, `rig = "*"`, `anyhow = "*"`, `serde = "*"`, `schemars = "*"`, `chrono = "*"`, etc. No version constraints anywhere. `Cargo.lock` exists but a fresh `cargo update` or `generate-lockfile` could pull breaking changes.
- **Plan:** Pin to current minor versions (e.g., `tokio = "1"`, `anyhow = "1"`, `rig = "0.11"`). Use `cargo update` deliberately, not accidentally.

### 7.2 [P2] `cargo nextest` not available in devShell
- **Where:** `flake.nix:116-122`
- **What:** `checks.nextest` uses `craneLib.cargoNextest` (which auto-adds `cargo-nextest`), but `devShells.default` does not include it. Developers cannot run `cargo nextest` locally.
- **Plan:** Add `cargo-nextest` to `devShells.default.packages`.

### 7.3 [P2] Rust toolchain not pinned in devShell
- **Where:** `flake.nix:119`
- **What:** Only `rustup` is provided. No `fenix`/`rust-overlay` input, no `rust-toolchain.toml`. Edition 2024 requires Rust >= 1.85.
- **Plan:** Use `fenix` or `rust-overlay` in the flake to pin a specific Rust version, or create a `rust-toolchain.toml` file.

### 7.4 [P2] Inconsistent dependency declaration pattern
- **Where:** `crates/tsukkomi/Cargo.toml:14,16,17,19,23`
- **What:** `serde`, `schemars`, `chrono`, `humantime`, `tempfile` use crate-local `"*"` instead of `workspace = true`. `chrono` is particularly wasteful — declared in workspace deps but overridden locally.
- **Plan:** Move all shared deps to `[workspace.dependencies]` and use `workspace = true` everywhere. For `chrono`, use `chrono = { workspace = true, default-features = false, features = ["std", "serde"] }`.

---

## 8. Testing Gaps

### 8.1 [P2] No integration tests for `ChatManager::reply`
- **Where:** `crates/tsukkomi/src/chat.rs` (8 inline unit tests exist, but no integration tests for the orchestration logic: debounce, image description, retry loop, compaction trigger).
- **Plan:** Full integration tests require a mock `CompletionModel` implementation, blocked by `CompletionModel` trait complexity (3 associated types + async methods). Deferred until a suitable test harness is available.

### 8.2 [P2] `cli.rs` has zero tests
- **Where:** `crates/tsukkomi/src/cli.rs`
- **What:** The CLI options module has no test coverage despite having sensible defaults and env-var fallbacks worth testing.
- **Plan:** Add smoke tests for default value parsing, env var overrides, and boundary values (e.g., `batch_size = 0`).

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
- **Where:** `nixos/tsukkomi.nix:91-95,181`
- **What:** `telegram.chats` is `listOf str`, but the CLI expects `Vec<i64>`. A non-numeric chat ID causes a runtime parse error at bot startup, not at `nixos-rebuild` time.
- **Plan:** Change the option type to `listOf int` (or add an assertion that each element parses as `i64`).

### 9.3 [P3] `AGENTS.md` conventions partially not enforced
- **Where:** Project root
- **What:** Conventions mention `cargo nextest run` and `cargo clippy`, but CI is Nix-based (`nix flake check`). New contributors may not realize `nix fmt` is required.
- **Plan:** Add a `CONTRIBUTING.md` that lists the pre-commit checklist and links to the Nix setup.

---

## 10. Newly Discovered Issues

### 10.1 [P2] Telegram: bot replies not threaded as replies
- **Where:** `crates/tsukkomi-telegram/src/main.rs:136`
- **What:** `bot.send_message(msg.chat.id, response.text)` sends a standalone message without `.reply_to_message_id(msg.id)`. In group chats, users won't see what the bot is responding to.
- **Plan:** Add `.reply_to_message_id(msg.id)` to the send call.

### 10.2 [P2] Matrix: send failures silently discarded
- **Where:** `crates/tsukkomi-matrix/src/main.rs:287`
- **What:** `let _ = room.send(content).await;` — if sending the AI reply fails (network error, permission denied, rate limit), the error is completely swallowed.
- **Plan:** At minimum, log the error: `if let Err(e) = room.send(content).await { tracing::error!("Failed to send reply: {e}"); }`.

### 10.3 [P3] Matrix: fixed 5-second retry with no exponential backoff
- **Where:** `crates/tsukkomi-matrix/src/main.rs:113-115`
- **What:** On persistent sync errors, the bot retries every 5 seconds indefinitely. Could hammer the server during outages.
- **Plan:** Add exponential backoff (e.g., `backoff` crate or manual doubling up to a max interval).

### 10.4 [P3] Matrix: no edited-message filtering (`m.replace`)
- **Where:** `crates/tsukkomi-matrix/src/main.rs:237-282`
- **What:** Edited messages arrive as new events with `m.relates_to` of type `m.replace`. The code never checks for this relation, so edits are processed as completely new messages — potentially causing duplicate or confusing AI responses.
- **Plan:** Check `event.content.relates_to` for `Relation::Replacement` and skip or handle accordingly.

### 10.5 [P3] Hardcoded Chinese in `remembering.rs`
- **Where:** `crates/tsukkomi/src/memory/remembering.rs:42`
- **What:** `format!("长期记忆：\n{summary}")` — the memory section header is hardcoded Chinese, inconsistent with the prompt files' bilingual pattern.
- **Plan:** Move to a prompt file or make configurable.

---

## Summary Table

| # | Issue | Priority | File(s) | Status |
|---|-------|----------|---------|--------|
| 1 | `std::sync::Mutex` in async | P0 | `chat.rs` | Open |
| 2 | `CURRENT_ROOM` task-local leak | P1 | `chat.rs`, `store.rs` | Open |
| 3 | Path traversal in filenames | P1 | `store.rs`, `file.rs` | Open |
| 4 | Corrupted `.jsonl` diagnostics | P1 | `file.rs` | Open |
| 5 | No message deduplication (Matrix) | P1 | `tsukkomi-matrix/main.rs` | Open |
| 6 | Telegram no startup filter | P2 | `tsukkomi-telegram/main.rs` | Open |
| 7 | `panic!` on missing prompt file | P2 | `chat.rs` | Open (by design) |
| 8 | `unwrap()` on `client.user_id()` | P2 | `tsukkomi-matrix/main.rs` | Open (by design) |
| 9 | Session parse failure silent | P2 | `tsukkomi-matrix/main.rs` | Open |
| 10 | Invalid timestamp silent drop | P2 | `tsukkomi-matrix/main.rs` | Open |
| 11 | `max_retries` returns `Ok(None)` | P2 | `chat.rs` | Open |
| 12 | Telegram token leaked in debug | P1 | `tsukkomi-telegram/main.rs` | Open |
| 13 | NixOS no systemd sandboxing | P2 | `nixos/tsukkomi.nix` | Open |
| 14 | Shared `StateDirectory` collision | P2 | `nixos/tsukkomi.nix` | Open |
| 15 | Schemas regenerated every call | P2 | `chat.rs` | Open |
| 16 | Hardcoded Chinese/English mix | P2 | `chat.rs` | Open |
| 17 | `ChatManager` unnecessary generics | P2 | `chat.rs` | Open |
| 18 | `compact_before_prompt` discarded return | P2 | `chat.rs` | Open |
| 19 | Default `batch_size` equals window | P2 | `cli.rs` | Open |
| 20 | `init_tracing` panics on double-init | P3 | `utils.rs` | Open |
| 21 | No rate limiting on LLM calls | P2 | `chat.rs` | Open |
| 22 | No health check endpoint | P2 | Both binaries | Open |
| 23 | Telegram stickers not handled | P2 | `tsukkomi-telegram/main.rs` | Open |
| 24 | Image understanding lacks caching | P2 | `chat.rs` | Open |
| 25 | Unbounded `*` dependencies | P1 | `Cargo.toml` | Open |
| 26 | `cargo nextest` missing in devShell | P2 | `flake.nix` | Open |
| 27 | Rust toolchain not pinned | P2 | `flake.nix` | Open |
| 28 | Inconsistent dep declarations | P2 | Various `Cargo.toml` | Open |
| 29 | No integration tests for `reply` | P2 | `chat.rs` | Partial |
| 30 | `cli.rs` has zero tests | P2 | `cli.rs` | Open |
| 31 | No README | P2 | Root | Open |
| 32 | NixOS `telegram.chats` type risk | P2 | `nixos/tsukkomi.nix` | Open |
| 33 | `AGENTS.md` conventions not enforced | P3 | Root | Open |
| 34 | Telegram replies not threaded | P2 | `tsukkomi-telegram/main.rs` | New |
| 35 | Matrix send failures discarded | P2 | `tsukkomi-matrix/main.rs` | New |
| 36 | Matrix fixed retry, no backoff | P3 | `tsukkomi-matrix/main.rs` | New |
| 37 | Matrix no edited-message filtering | P3 | `tsukkomi-matrix/main.rs` | New |
| 38 | Hardcoded Chinese in `remembering.rs` | P3 | `memory/remembering.rs` | New |

**Fixed since initial review (removed from tracking):**
- Non-atomic file writes → `atomic_write` in `memory/utils.rs`
- `std::sync::Mutex` in memory lock registries → `tokio::sync::Mutex` (commit `7245df4`)
- Matrix recovery key failure silent → now logs `warn!`
- Matrix password in debug log → now logs only `.is_some()`
- `MessageBody::Image` unused variant → refactored to `ImageData` + `describe_images()`
- `_opts` unused in Telegram handler → comment added (PR #1)
- Tests for `compactor.rs` → 4 tests added
- Tests for `file.rs` → 9 tests added
- Binary CLI smoke tests → Matrix: 4, Telegram: 3

**Current test count: 46** (core: 39, matrix: 4, telegram: 3)