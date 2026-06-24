# AGENTS.md

## Project

Telegram bot — an LLM-powered "tsukkomi" (吐槽役) that participates in group chats to keep conversations lively. Written in Rust.

## Develop environment

- Managed by Nix flake + direnv. Run `direnv allow` before first use.
- If `nix develop` or `direnv` fails, check `flake.nix` for the expected inputs and system requirements.
- Rust toolchain version is pinned in the Nix flake, not via `rust-toolchain.toml`.
- Use `nix develop -- COMMAND` to run commands in the dev environment (e.g., `nix develop -- cargo test`).
- If a tool is missing from the dev environment, add it to `devShells.default` in `flake.nix` (ask before adding).

## Commands

```bash
nix flake check      # run all checks (fmt, build, clippy, tests, docs)
nix run .#tsukkomi   # run the bot locally
```

## Docs

- Use `cargo doc` to generate project and dependency documentation (HTML) to `target/doc` for reference.
- Use `nix develop -- cargo doc` to generate docs in the dev environment.

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
- Telegram bot framework: teloxide.
