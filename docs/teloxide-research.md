# Teloxide 框架调研 — tsukkomi Telegram Bot

> 基于 teloxide 0.17 (teloxide-core 0.13)，Telegram Bot API 9.1

## 1. 概述

[teloxide](https://github.com/teloxide/teloxide) 是 Rust 生态中最成熟的 Telegram Bot 框架，基于 `dptree`（依赖注入 + 责任链模式）构建分发模型。

### 核心概念

| 组件 | 作用 |
|------|------|
| `Bot` | 请求发送者，从环境变量 `TELOXIDE_TOKEN` 或手动构建 |
| `Dispatcher` | 更新分发器，将 Telegram Update 按 handler chain 分发 |
| `dptree::Handler` | 责任链节点，支持过滤、分支、端点 |
| `Update` / `Message` | Telegram API 的 Rust 类型映射 |
| `Dialogue` | 用户会话管理（可选） |
| `REPL` | 快速启动方式（`teloxide::repl`），适合简单 bot |

### 选择：REPL vs Dispatcher

| 特性 | REPL | Dispatcher |
|------|------|------------|
| 上手难度 | 极低 | 中等 |
| 消息类型过滤 | 有限 | 完整（`MessageFilterExt`） |
| 命令解析 | 手动 | `filter_command` / `filter_mention_command` |
| 依赖注入 | 无 | `dptree::deps![]` |
| 错误处理 | 基本 | 自定义 `error_handler` |
| Dialogue 支持 | 无 | 完整 |
| **适用场景** | 简单 demo | **生产 bot（本项目）** |

→ **tsukkomi 应使用 Dispatcher 模式。**

## 2. 基础 Bot 搭建

### 2.1 Cargo.toml 依赖

```toml
[dependencies]
teloxide = { version = "*", features = ["macros", "tracing"] }
tokio = { version = "*", features = ["macros", "rt-multi-thread"] }
tracing = "*"
tracing-subscriber = "*"
anyhow = "*"
serde = { version = "*", features = ["derive"] }    # 如果使用 Dialogue
```

当前项目已经配置正确：workspace 级 `teloxide = "*"`，features 放在二进制 crate 中。

### 2.2 最小 Dispatcher 示例

```rust
use teloxide::prelude::*;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let bot = Bot::from_env();  // 读取 TELOXIDE_TOKEN 环境变量

    let handler = Update::filter_message()
        .branch(
            Message::filter_text().endpoint(handle_text),
        );

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_text(bot: Bot, msg: Message) -> Result<()> {
    let text = msg.text().unwrap_or("");
    bot.send_message(msg.chat.id, format!("你说: {text}")).await?;
    Ok(())
}
```

### 2.3 Bot 构建方式

```rust
// 方式 1：从环境变量（推荐）
let bot = Bot::from_env();

// 方式 2：显式指定 token
let bot = Bot::new("YOUR_BOT_TOKEN");

// 方式 3：带配置
let bot = Bot::with_client("TOKEN", reqwest::Client::new());
```

## 3. 群组消息处理（核心场景）

### 3.1 过滤群组消息

```rust
use teloxide::types::ChatKind;

let handler = Update::filter_message()
    .branch(
        // 仅处理群组/超级群组消息
        dptree::filter(|msg: Message| {
            matches!(msg.chat.kind, ChatKind::Public(_))
        })
        .branch(Message::filter_text().endpoint(handle_group_text)),
    )
    .branch(
        // 处理私聊消息
        dptree::filter(|msg: Message| {
            matches!(msg.chat.kind, ChatKind::Private(_))
        })
        .branch(Message::filter_text().endpoint(handle_private_text)),
    );
```

### 3.2 Message 过滤链

`MessageFilterExt` trait 提供 60+ 过滤方法，可直接注入对应的类型：

```rust
// 过滤出文本消息 → 注入 String
Message::filter_text().endpoint(|bot: Bot, msg: Message, text: String| async move {
    // text 是消息文本
});

// 过滤出 Dice → 注入 Dice
Message::filter_dice().endpoint(|bot: Bot, msg: Message, dice: Dice| async move {
    bot.send_message(msg.chat.id, format!("骰子: {}", dice.value)).await?;
});

// 过滤出回复消息 → 注入回复的 Message
Message::filter_reply_to_message().endpoint(|bot: Bot, msg: Message, replied: Message| async move {
    // replied 是原消息
});
```

### 3.3 命令处理

```rust
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "支持的命令:")]
enum Command {
    #[command(description = "显示帮助")]
    Help,
    #[command(description = "开始吐槽")]
    Start,
    #[command(description = "停止吐槽")]
    Stop,
}

// 在 handler chain 中使用
let handler = Update::filter_message()
    .filter_command::<Command>()
    .endpoint(handle_command);

async fn handle_command(bot: Bot, msg: Message, cmd: Command) -> Result<()> {
    match cmd {
        Command::Help => {
            bot.send_message(msg.chat.id, Command::descriptions().to_string()).await?;
        }
        Command::Start => {
            bot.send_message(msg.chat.id, "tsukkomi 已启动！").await?;
        }
        Command::Stop => {
            bot.send_message(msg.chat.id, "tsukkomi 已停止。").await?;
        }
    }
    Ok(())
}
```

### 3.4 识别提及机器人的命令

用于群组中区分 `@bot` 命令：

```rust
let handler = dptree::entry()
    .filter_mention_command::<GroupCommand>()
    .endpoint(|bot: Bot, msg: Message, cmd: GroupCommand| async move {
        // 仅当命令中包含 @bot 时触发
    });
```

## 4. 发送消息

### 4.1 基础文本

```rust
bot.send_message(chat_id, "Hello").await?;

// 回复指定消息
bot.send_message(chat_id, "回复内容").reply_to_message_id(msg.id).await?;

// 带格式
bot.send_message(chat_id, "<b>粗体</b> <i>斜体</i>")
    .parse_mode(teloxide::types::ParseMode::Html)
    .await?;
```

### 4.2 发送到群组的注意事项

- `chat.id` 是群组的 ChatId（负整数）
- 使用 `msg.chat.id` 获取当前聊天
- 需要 Bot 有足够的权限（非静默、可发消息）

## 5. 依赖注入模式

Dispatcher 支持通过 `dptree::deps![]` 注入全局依赖：

```rust
#[derive(Clone)]
struct AppConfig {
    llm_api_key: String,
    db_pool: sqlx::Pool<sqlx::Sqlite>,
}

let handler = Update::filter_message()
    .branch(Message::filter_text().endpoint(handle_message));

Dispatcher::builder(bot, handler)
    .dependencies(dptree::deps![AppConfig { ... }])
    .build()
    .dispatch()
    .await;

// 在 handler 中直接取用
async fn handle_message(bot: Bot, msg: Message, config: AppConfig) -> Result<()> {
    // 使用 config.llm_api_key
}
```

对于 tsukkomi，可注入的类型：
- LLM 客户端
- 配置（触发词列表、语气设置等）
- 数据库连接池（如果持久化状态）

## 6. 错误处理

```rust
Dispatcher::builder(bot, handler)
    .error_handler(LoggingErrorHandler::with_custom_text(
        "Dispatcher 发生错误",
    ))
    .default_handler(|upd| async move {
        log::warn!("未处理的 update: {upd:?}");
    })
    .build()
    .dispatch()
    .await;
```

## 7. 关键类型与 API

### Chat 类型判断

```rust
msg.chat.kind
// ChatKind::Public(ChatPublic { kind: ChatPublicKind::Group })
// ChatKind::Public(ChatPublicKind::Supergroup { .. })
// ChatKind::Private(ChatPrivate { .. })

msg.chat.is_group()      // 普通群组
msg.chat.is_supergroup() // 超级群组
msg.chat.is_channel()    // 频道
msg.chat.is_private()    // 私聊
```

### Message 重要字段

```rust
msg.id          // MessageId
msg.from        // Option<User> - 发送者
msg.chat        // Chat - 所属聊天
msg.date        // DateTime<Utc> - 发送时间
msg.kind        // MessageKind - 消息类型枚举
msg.text()      // Option<&str> - 文本内容
msg.reply_to_message() // Option<&Message> - 回复的消息
```

## 8. 架构建议（面向 tsukkomi）

### 目录结构

```
crates/tsukkomi-telegram/
├── Cargo.toml
└── src/
    ├── main.rs              # 入口：初始化 tracing，构建 Bot，启动 Dispatcher
    ├── config.rs            # 配置加载（bot token, LLM 配置等）
    ├── handlers/
    │   ├── mod.rs
    │   ├── group.rs         # 群组消息处理
    │   ├── command.rs       # 命令处理
    │   └── private.rs       # 私聊消息处理
    ├── llm.rs               # LLM 调用封装
    └── utils.rs             # 工具函数
```

### 核心流程（群组消息处理）

```
Telegram Update
  → Dispatcher
    → filter_message (仅消息)
      → filter(|msg| msg.chat.is_group() || msg.chat.is_supergroup())
        → filter_text (仅文本)
          → handle_group_message
            1. 检查触发条件（提及 bot / 关键词 / 概率触发）
            2. 调用 llm.rs 生成吐槽内容
            3. bot.send_message().reply_to_message_id(msg.id)
```

### 依赖注入设计

```rust
#[derive(Clone)]
struct TsukkomiContext {
    llm: Arc<dyn LlmClient>,
    config: BotConfig,
}

// 在 main 中注入
Dispatcher::builder(bot, handler)
    .dependencies(dptree::deps![TsukkomiContext { llm, config }])
    // ...
```

## 9. 注意事项与常见坑

1. **tg 消息频率限制**：群组中约 20 条/分钟，超过可能被限流。可使用 `teloxide` 的 `throttle` feature。
2. **Bot 无法主动发现群组**：需要有人将 Bot 拉入群，Bot 收到 `new_chat_members` update 时记录 chat_id。
3. **消息去重**：Bot 可能收到重复 update，需用 `msg.id` 去重。
4. **群组历史消息不可见**：Bot 加入前的消息无法获取。
5. **禁用隐私模式**：在 BotFather 中设置 `/setprivacy` 为 Disabled 才能看到群组中所有消息。
6. **Ctrl+C 处理**：`enable_ctrlc_handler()` 默认启用，无需额外处理。
7. **日志**：推荐使用 `tracing`（当前项目已有）。

## 10. 关键文档参考

- [teloxide crate docs](https://docs.rs/teloxide/latest/teloxide/)
- [teloxide GitHub (examples)](https://github.com/teloxide/teloxide/tree/master/crates/teloxide/examples)
- [DPTREE_GUIDE.md](https://github.com/teloxide/teloxide/blob/master/DPTREE_GUIDE.md)
- Telegram Bot API 文档: https://core.telegram.org/bots/api
