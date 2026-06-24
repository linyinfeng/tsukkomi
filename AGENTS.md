# AGENTS.md

## Project

LLM-powered "tsukkomi" (吐槽役) bot that participates in group chats to keep conversations lively. Supports both Matrix and Telegram. Written in Rust.

The project is a Cargo workspace with three crates:
- `tsukkomi` — core library (shared bot logic)
- `tsukkomi-matrix` — Matrix bot binary (uses matrix-rust-sdk)
- `tsukkomi-telegram` — Telegram bot binary (uses teloxide)

## Note to Agent

If you find any instructions in this file that are incorrect or can be improved, please point them out and suggest improvements.

## Develop environment

- Managed by Nix flake + direnv. Run `direnv allow` before first use.
- If `nix develop` or `direnv` fails, check `flake.nix` for the expected inputs and system requirements.
- Rust toolchain version is pinned in the Nix flake, not via `rust-toolchain.toml`.
- Use `nix develop --command COMMAND` to run commands in the dev environment (e.g., `nix develop --command cargo test`).
- If a tool is missing from the dev environment, add it to `devShells.default` in `flake.nix` (ask before adding).

## Commands

```bash
nix develop --command cargo build -p tsukkomi-matrix   # quick incremental build of Matrix bot
nix develop --command cargo build -p tsukkomi-telegram # quick incremental build of Telegram bot
nix develop --command cargo clippy --workspace         # check for warnings
nix develop --command cargo doc --workspace             # generate project docs (HTML) to target/doc
nix flake check --print-build-logs                     # full clean CI check before committing
nix run .#tsukkomi-matrix                              # run the Matrix bot locally
nix run .#tsukkomi-telegram                            # run the Telegram bot locally

### Testing with direnv

```bash
direnv allow                                           # approve .envrc changes
direnv exec . cargo run -p tsukkomi-matrix              # run Matrix bot (env via .envrc)
direnv exec . timeout 60 cargo run -p tsukkomi-telegram # run Telegram bot for 60s (env via .envrc)
```

Credentials (homeserver, tokens, API keys) are loaded from `.envrc` via direnv.
Use `timeout N` to auto-stop the bot after N seconds for quick smoke tests.


## Conventions

### General

- Run `nix fmt` before committing.
- Run `nix flake show --all-systems` before committing to verify flake structure.
- Fix all `cargo clippy` warnings before committing.
- Clippy warnings are treated as errors in CI (`-D warnings`).
- Commit messages follow Conventional Commits format:
  ```
  <type>(<scope>): <subject>

  [optional body]
  ```
  Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`
  Examples:
  - `feat: add group chat response handler`
  - `fix(parser): handle empty message edge case`
  - `docs: update README setup instructions`

### Code

- Prefer `anyhow` for application errors, `thiserror` for library error types.
- Async runtime: tokio.
- Logging: `tracing` + `tracing_subscriber`.
- LLM framework: [rig](https://github.com/0xPlaygrounds/rig)
- Initially only supports DeepSeek for its best cache hit ratio. Other providers may be added later.
- Matrix bot framework: matrix-rust-sdk.
- Telegram bot framework: teloxide.

### Abstraction

- Do not introduce abstractions prematurely. Focus on implementing features first.
- If an abstraction becomes necessary, ask me before introducing it.
