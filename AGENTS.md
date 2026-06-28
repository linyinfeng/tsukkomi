# AGENTS.md — AI Agent 入口

## 项目概述

LLM-powered "tsukkomi" (吐槽役) bot，参与群聊保持活跃氛围。支持 Matrix 和 Telegram。Rust 编写。

## 架构

Cargo workspace，三个 crate：
- `tsukkomi` — 核心库（共享 bot 逻辑）
- `tsukkomi-matrix` — Matrix bot（matrix-rust-sdk）
- `tsukkomi-telegram` — Telegram bot（teloxide）

共享逻辑在 `tsukkomi::chat::ChatManager`（`crates/tsukkomi/src/chat.rs`）。

### 记忆系统

每个聊天室一个 `.jsonl` 文件（`memory/` 目录）。滑动窗口超限时，旧消息分批降级并由 `TsukkomiCompactor` 压缩为摘要。

## 开发环境

- Nix flake + direnv 管理
- Rust 工具链版本在 `flake.nix` 中固定
- 用 `nix develop --command COMMAND` 运行命令

## 常用命令

```bash
nix develop --command cargo build -p tsukkomi-matrix
nix develop --command cargo build -p tsukkomi-telegram
nix develop --command cargo clippy --workspace
nix develop --command cargo test --workspace
nix develop --command cargo doc --workspace
nix flake check --print-build-logs
```

## 约定

### 提交规范

- 提交前运行 `nix fmt`
- 提交前运行 `nix flake show --all-systems` 验证 flake 结构
- 修复所有 clippy 警告（CI 中 `-D warnings`）
- Conventional Commits 格式：`<type>(<scope>): <subject>`
- Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

### 代码规范

- 应用错误用 `anyhow`，库错误用 `thiserror`
- Async: tokio
- 日志: tracing + tracing_subscriber
- LLM: rig 框架，DeepSeek 主模型（`DEEPSEEK_V4_FLASH`），图像理解 MiMo（`MIMO_V2_5`）
- 不要过早引入抽象

## 凭证

从 `.envrc` 加载（direnv）：
- `MATRIX_HOMESERVER`, `MATRIX_USERNAME`, `MATRIX_PASSWORD`, `MATRIX_ROOMS`
- `TELOXIDE_TOKEN`, `TELEGRAM_CHATS`
- `DEEPSEEK_API_KEY`
- `XIAOMI_MIMO_API_KEY`

## 目录约定

- 主分支为 `main`，开发分支为 `develop`
- 开发功能或做修复都在独立 worktree 中进行
- worktree 统一建立在 `../tsukkomi-worktrees/`
