# TODO

## Bugs & Data Safety

- ~~**`crates/tsukkomi/src/memory/store.rs:109`** — `serde_json::from_str(&content).unwrap_or_default()` silently discards all stored memories on parse failure. Corrupted JSON will wipe the memories file. Should log a warning and/or back up the original file before overwriting.~~ **FIXED** — now uses `?` to propagate the error upward.
- ~~**`crates/tsukkomi/src/memory/store.rs:100`** — `file.lock()` is a synchronous `flock` syscall inside an async context. Although brief, it blocks the tokio runtime. Use traits in `fs4::tokio` to do async locks.~~ **FIXED** — now uses `async_fd_lock::{LockRead, LockWrite}`.

## Missing Features

- ~~**No tests** — `cargo nextest run` finds zero tests in the workspace. Core modules that should have tests: `window.rs` (sliding window logic), `chat.rs` (prompt building), `memory/store.rs` (remember/forget/modify).~~ **FIXED** — `window.rs`: 8 tests, `store.rs`: 7 tests, `chat.rs`: 3 tests.
- **`chat.rs`: `MessageBody::Image { url }`** — Image message variant is defined and included in the JSON schema presented to the LLM, but neither `tsukkomi-matrix` nor `tsukkomi-telegram` ever produces an `Image` body. The LLM schema and the actual input are inconsistent.
- **No message deduplication** — `tsukkomi-matrix` skips old messages by comparing `origin_server_ts` to startup time, but after a sync restart or reconnection within the same session, already-processed messages could be re-fed to the LLM.
- **No graceful shutdown** — `tsukkomi-matrix` runs an infinite sync loop with no signal handler; `tsukkomi-telegram` has `enable_ctrlc_handler()` via teloxide, but the Matrix bot does not.

## Potential Issues

- **`tsukkomi-telegram/main.rs:82` `_opts` unused in `msg_handler`** — the `Arc<Options>` is passed via dependency injection but never read. If it's truly unnecessary, it can be removed; if it's reserved for future use, add a comment.

## Code Quality

- ~~**`tsukkomi-matrix/main.rs:215`** — startup timestamp uses `Utc::now().timestamp_millis()` as `StartupTime(i64)`, but `origin_server_ts` also returns milliseconds. This works correctly but could use an explicit type alias or doc comment to clarify the unit.~~ **FIXED** — `StartupTime` now wraps `DateTime<Utc>`; `origin_server_ts` is converted to `DateTime<Utc>` for comparison.
- ~~**Consider extracting the common bot pattern** (receive message → build payload → call `manager.reply()` → send response) into a shared helper in the core crate to reduce duplication between Matrix and Telegram main files.~~ Not planned — the two bots have different enough event types and SDKs that a shared abstraction would add complexity without meaningful reuse.
