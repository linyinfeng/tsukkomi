# matrix-rust-sdk 调研 — tsukkomi Matrix Bot

> 基于 matrix-sdk 0.18，ruma 0.16，Matrix Client-Server API v1.12+

## 1. 概述

[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk) 是 Matrix 官方维护的 Rust SDK，提供了完整的 Client-Server API 实现。与 teloxide 不同，Matrix SDK 不提供"bot 框架"的概念，而是提供底层客户端，开发者需自行构建事件驱动的循环。

### 核心概念

| 组件 | 作用 |
|------|------|
| `Client` | 核心客户端（内部 Arc，可 Clone） |
| `ClientBuilder` | 构建 Client（配置 store、homeserver 等） |
| `Room` | 房间抽象（Joined / Invited / Left 三种状态） |
| `SyncRoomMessageEvent` | 同步到的房间消息事件 |
| `ruma` | Matrix 协议类型库（事件类型、标识符等） |
| `StateStore` | 状态持久化（SQLite 内置支持） |
| `EventCache` | 事件缓存层，提供事件订阅 API |
| `SlidingSync` | 增量同步（MSC3575，新一代同步方式） |

### 与 teloxide 的关键区别

| 方面 | teloxide | matrix-rust-sdk |
|------|----------|-----------------|
| 抽象层次 | 高（bot 框架） | 中（客户端 SDK） |
| 回调模式 | `Dispatcher` / `REPL` | `add_event_handler` + `sync` |
| 消息接收 | 长轮询 / Webhook | 持久连接 Sync |
| 认证 | Bot Token | 用户名密码 / Access Token / Appservice |
| 状态管理 | Dialogue（可选） | StateStore（必须） |
| 端到端加密 | 不涉及 | 内置支持（`e2e-encryption` feature） |
| 重启恢复 | 无状态 | 自动通过 StateStore 恢复 sync token |

## 2. 基础 Bot 搭建

### 2.1 Cargo.toml 依赖

```toml
[dependencies]
matrix-sdk = { version = "*", features = ["sqlite", "markdown"] }
tokio = { version = "*", features = ["macros", "rt-multi-thread"] }
tracing = "*"
tracing-subscriber = "*"
anyhow = "*"
```

当前项目已配置 `matrix-sdk = "*"` 在 workspace 依赖中。建议显式启用 `sqlite` 和可选的 `markdown`。

### 2.2 完整 Bot 示例（含 Session 持久化）

```rust
use matrix_sdk::{
    Client, Room, config::SyncSettings,
    ruma::events::room::message::SyncRoomMessageEvent,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // 1. 创建 Client（SQLite 持久化）
    let client = Client::builder()
        .server_name("matrix.example.org")
        .sqlite_store("./store", None)
        .build()
        .await?;

    // 2. 尝试恢复 session 或登录
    if let Ok(session) = load_session_from_disk() {
        client.restore_session(session).await?;
        tracing::info!("已恢复 session: {}", client.user_id().unwrap());
    } else {
        client.matrix_auth()
            .login_username("@bot:example.org", "hunter2")
            .send()
            .await?;
        // 保存 session 供下次使用
        save_session_to_disk(client.session().unwrap()).await?;
        tracing::info!("已登录为: {}", client.user_id().unwrap());
    }

    // 3. 注册事件处理器
    client.add_event_handler(handle_room_message);
    client.add_event_handler(handle_invite);

    // 4. 开始同步
    client.sync(SyncSettings::default()).await?;

    Ok(())
}

async fn handle_room_message(
    ev: SyncRoomMessageEvent, room: Room, client: Client,
) {
    // 跳过自己的消息
    if ev.sender().map_or(true, |s| s == *client.user_id().unwrap()) {
        return;
    }

    // 提取文本
    if let Some(content) = ev.as_original() {
        if let matrix_sdk::ruma::events::room::message::MessageType::Text(t) = &content.content.msgtype {
            tracing::info!("[{}] {}: {}", room.room_id(), ev.sender().unwrap(), t.body);
            // 处理消息...
        }
    }
}

async fn handle_invite(
    ev: matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent,
    room: Room,
) {
    if room.state() == matrix_sdk::RoomState::Invited {
        tracing::info!("收到邀请: {}", room.room_id());
        if let Err(e) = room.join().await {
            tracing::error!("加入房间失败: {e}");
        }
    }
}
```

### 2.3 使用 Session Token 恢复登录（推荐）

```rust
// 首次登录后保存 session
let session = client.session().unwrap();
let json = serde_json::to_string(&session)?;
std::fs::write("session.json", json)?;

// 后续启动时恢复
let json = std::fs::read_to_string("session.json")?;
let session: matrix_sdk::AuthSession = serde_json::from_str(&json)?;
client.restore_session(session).await?;
```

> 注意：`AuthSession` 实现了 `Serialize`/`Deserialize`，可直接 JSON 序列化保存。

## 3. 消息监听与处理（核心场景）

### 3.1 事件处理器签名

```rust
// 基本形式：事件类型 + 上下文参数
client.add_event_handler(|ev: SyncRoomMessageEvent| async move { });

// 常用：事件 + 房间 + 客户端
client.add_event_handler(|ev: SyncRoomMessageEvent, room: Room, client: Client| async move { });

// 额外上下文：加密信息
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, encryption_info: Option<EncryptionInfo>| async move { }
);

// 额外上下文：push actions（是否高亮等）
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, push_actions: Vec<Action>| async move { }
);

// 自定义上下文（使用 Ctx<T>）
#[derive(Clone)]
struct AppState { db_pool: sqlx::Pool<sqlx::Sqlite> }

client.add_event_handler_context(AppState { db_pool });
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, state: Ctx<AppState>| async move { }
);
```

### 3.2 过滤消息（提取文本）

```rust
use matrix_sdk::ruma::events::{
    room::message::{SyncRoomMessageEvent, MessageType},
    AnySyncMessageLikeEvent,
};

client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room, client: Client| async move {
        // 跳过自己的消息
        if ev.sender().map_or(true, |s| s == *client.user_id().unwrap()) {
            return;
        }

        // 获取原始消息内容
        let Some(AnySyncMessageLikeEvent::Original(original)) = ev.original_content() else {
            return;  // 跳过编辑后的消息
        };

        // 按消息类型处理
        match &original.content.msgtype {
            MessageType::Text(t) => {
                let text = &t.body;
                // 处理文本消息
            }
            MessageType::Emote(e) => {
                let text = &e.body;
                // 处理 /me 动作
            }
            MessageType::Notice(n) => {
                let text = &n.body;
                // 处理通知消息（通常 Bot 消息标记为 notice）
            }
            _ => {} // 图片、文件等其他类型跳过
        }
    },
);
```

> `MessageType::Emote` 对应 Matrix 的 `m.emote`，类似 IRC 的 `/me` 动作。`MessageType::Notice` 对应 `m.notice`，通常由 Bot 发出。

### 3.3 判断群组 vs 私聊

```rust
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room| async move {
        if room.is_dm() {
            // 私聊：只有两个人的房间
            handle_dm(ev, room).await;
        } else if room.is_space() {
            // Spaces（空间），跳过
        } else {
            // 群组聊天
            handle_group_message(ev, room).await;
        }
    },
);
```

### 3.4 特定的房间监听

```rust
// 方式 1：只为特定房间注册 handler
client.add_room_event_handler(
    room_id!("!roomid:example.org"),
    |ev: SyncRoomMessageEvent, room: Room| async move {
        // 仅在该房间中触发
    },
);

// 方式 2：通过 Room 对象注册
if let Some(room) = client.get_room(&room_id!("!roomid:example.org")) {
    room.add_event_handler(|ev: SyncRoomMessageEvent| async move {
        // 仅在该房间中触发
    });
}
```

### 3.5 事件过滤器（主动筛选已加入的房间）

```rust
// 只处理已加入房间的消息
client.add_event_handler(
    |ev: SyncRoomMessageEvent, room: Room| async move {
        if room.state() != matrix_sdk::RoomState::Joined {
            return;
        }
        // 处理消息...
    },
);
```

## 4. 发送消息

### 4.1 基础文本消息

```rust
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;

// 发送纯文本
let content = RoomMessageEventContent::text_plain("Hello from tsukkomi!");
room.send(content).await?;

// 发送通知（m.notice，通常 Bot 使用）
let content = RoomMessageEventContent::notice_plain("Bot 消息");
room.send(content).await?;

// 发送 Markdown（需要 `markdown` feature）
let content = RoomMessageEventContent::text_markdown("**粗体** *斜体*");
room.send(content).await?;

// 发送 HTML 格式化消息
use matrix_sdk::ruma::events::room::message::{
    RoomMessageEventContent, TextMessageEventContent, Format, FormattedBody,
};
let content = RoomMessageEventContent::text_html(
    "粗体 斜体",
    "粗体 <i>斜体</i>",
);
room.send(content).await?;
```

### 4.2 回复消息

```rust
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::room::reply::Reply;

// 创建回复事件（需要原始事件来构建回复结构）
let reply = Reply::new(&ev);
let reply_content = room.make_reply_event(
    RoomMessageEventContent::text_plain("这是回复"),
    reply,
).await?;

room.send(reply_content).await?;
```

> 注意：`Reply::new` 的参数是实现了 `ReplyToEvent` trait 的类型。`SyncRoomMessageEvent` 满足此 trait。

### 4.3 发送 typing 指示（LLM 处理时使用）

```rust
// 开始 typing
room.typing_notice(true).await?;

// LLM 处理...

// 停止 typing
room.typing_notice(false).await?;
```

### 4.4 获取房间引用

```rust
// 方式 1：从事件处理器中获取
client.add_event_handler(|ev: SyncRoomMessageEvent, room: Room| async move {
    room.send(content).await?;
});

// 方式 2：通过 room_id 获取
let room = client.get_room(&room_id!("!roomid:example.org")).unwrap();

// 方式 3：遍历所有已加入房间
for room in client.joined_rooms() {
    // room 类型为 Room，state() 为 Joined
}
```

## 5. 同步策略

### 5.1 经典 Sync（推荐用于 Bot）

```rust
use matrix_sdk::config::SyncSettings;
use std::time::Duration;

// 简单同步（永不返回）
client.sync(SyncSettings::default()).await?;

// 自定义同步
let settings = SyncSettings::default()
    .timeout(Duration::from_secs(30))  // 长轮询超时
    .token("previous_batch_token");    // 从指定 token 开始

client.sync_with_callback(settings, |response| async move {
    tracing::info!("收到 sync 响应");
}).await?;
```

### 5.2 Sync 在后台 Task 中运行

```rust
// 将 sync 放入后台任务
let sync_client = client.clone();
tokio::spawn(async move {
    loop {
        if let Err(e) = sync_client.sync(SyncSettings::default()).await {
            tracing::error!("Sync 错误: {e}，将在 5 秒后重试...");
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }
});
```

> SDK 内部有重连逻辑，但如果 sync 函数返回错误，需要手动重试循环。

### 5.3 Sliding Sync（实验性，需要服务器支持）

```rust
// Sliding Sync 是一种增量同步协议（MSC3575）
// 需要 homeserver 支持（如 Synapse 1.100+、Conduit）
// 适用于需要按列表筛选房间的场景

use matrix_sdk::sliding_sync::SlidingSyncMode;

let sliding_sync = client.sliding_sync("tsukkomi")?
    .add_list(
        SlidingSyncList::builder("all_rooms")
            .sync_mode(SlidingSyncMode::new_selective()
                .add_range(0, 100)),
    )
    .build()
    .await?;
```

**建议**：Bot 场景使用经典 Sync 更简单可靠，Sliding Sync 适合移动端或需要精细控制同步列表的场景。

### 5.4 Event Cache 订阅模式

```rust
// 启用 event cache 后，可以订阅房间事件流
let event_cache = client.event_cache();
let room_cache = event_cache.for_room(room.room_id()).await?;
let mut subscriber = room_cache.subscribe().await?;

// 使用 Stream 模式处理事件
use futures_util::StreamExt;
while let Some(update) = subscriber.next().await {
    // 处理房间事件更新
    tracing::info!("房间更新: {:?}", update);
}
```

## 6. 自动接受邀请

```rust
use matrix_sdk::ruma::events::room::member::StrippedRoomMemberEvent;
use matrix_sdk::RoomState;

client.add_event_handler(
    |ev: StrippedRoomMemberEvent, room: Room, client: Client| async move {
        // 确认邀请是针对 bot 自己的
        if ev.state_key() != client.user_id().unwrap() {
            return;
        }

        if room.state() == RoomState::Invited {
            tracing::info!("被邀请到房间: {} ({})", room.room_id(), room.display_name().await.unwrap_or_default());
            match room.join().await {
                Ok(()) => tracing::info!("已加入房间: {}", room.room_id()),
                Err(e) => tracing::error!("加入房间失败 {}: {e}", room.room_id()),
            }
        }
    },
);
```

## 7. 状态持久化

### 7.1 SQLite 存储

```rust
let client = Client::builder()
    .server_name("example.org")
    .sqlite_store("./store", None)  // 数据存储在 ./store/
    .build()
    .await?;
```

SQLite 会存储：
- 房间状态（名称、主题、成员列表等）
- 同步 token（保证断线重连后不丢消息）
- E2EE 密钥（如果启用 `e2e-encryption`）

### 7.2 内存存储（测试用）

```rust
let client = Client::builder()
    .server_name("example.org")
    .build()
    .await?;  // 默认 MemoryStore
```

**Bot 必须使用 SQLite 或类似持久化存储**，否则每次重启都需要全量同步，且在加密房间中会丢失密钥。

## 8. 端到端加密（E2EE）

`e2e-encryption` 默认启用（dev 依赖中 feature 是默认开启的）。对于 Bot 场景：

```rust
let client = Client::builder()
    .server_name("example.org")
    .sqlite_store("./store", Some("passphrase"))  // 加密存储
    .build()
    .await?;

// 检查房间加密状态
let encryption_state = room.encryption_state();
match encryption_state {
    EncryptionState::Encrypted => {
        // E2EE 房间，SDK 自动解密
        // 解密后的 SyncRoomMessageEvent 的 original_content() 可用
    }
    EncryptionState::NotEncrypted => {
        // 明文消息
    }
    EncryptionState::Unknown => {
        room.request_encryption_state().await?;
    }
}
```

**关键注意**：
- 如果 Bot 加入的 Matrix 群组启用了 E2EE，必须启用 `e2e-encryption` feature 才能读取消息内容。
- SQLite store 开启 passphrase 保护 E2EE 密钥。
- Bot 的第一次同步在加密房间中可能无法立即解密历史消息（需要先获取密钥）。

## 9. 架构建议（面向 tsukkomi）

### 目录结构

```
crates/tsukkomi-matrix/
├── Cargo.toml
└── src/
    ├── main.rs              # 入口：初始化 Client，登录/恢复，注册 handler，启动 sync
    ├── config.rs            # 配置加载（homeserver, credentials, LLM 配置）
    ├── handlers/
    │   ├── mod.rs
    │   ├── message.rs       # 消息事件处理
    │   └── invite.rs        # 邀请处理（自动加入房间）
    ├── llm.rs               # LLM 调用封装
    ├── session.rs           # Session 序列化/反序列化
    └── utils.rs             # 工具函数
```

### 核心流程

```
main()
  ├── 创建 Client（SQLite store）
  ├── 恢复/登录 Session
  ├── 注册事件处理器
  │   ├── handle_room_message
  │   │   1. 跳过自己的消息（sender == bot id）
  │   │   2. 提取文本 (MessageType::Text)
  │   │   3. 检查触发条件
  │   │   4. typing_notice(true)
  │   │   5. 调用 LLM 生成吐槽
  │   │   6. room.send() 或回复
  │   │   7. typing_notice(false)
  │   └── handle_invite
  │       1. 检查是 bot 的邀请
  │       2. room.join()
  └── 启动 sync（后台 task，带重试）
```

### 完整消息处理 handler

```rust
use matrix_sdk::{
    Client, Room,
    ruma::events::room::message::{
        SyncRoomMessageEvent, MessageType, RoomMessageEventContent,
    },
};

async fn handle_room_message(
    ev: SyncRoomMessageEvent,
    room: Room,
    client: Client,
) {
    // 1. 跳过自己的消息
    let Some(sender) = ev.sender() else { return };
    if sender == client.user_id().unwrap() {
        return;
    }

    // 2. 提取文本
    let Some(original) = ev.as_original() else { return };
    let MessageType::Text(text_content) = &original.content.msgtype else { return };

    // 3. 触发条件检查
    if !should_respond(&text_content.body) {
        return;
    }

    // 4. 显示 typing 指示
    let _ = room.typing_notice(true).await;

    // 5. LLM 调用（带超时）
    let response = match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        llm_client.generate(text_content.body),
    ).await {
        Ok(Ok(text)) => text,
        _ => {
            let _ = room.typing_notice(false).await;
            return;
        }
    };

    // 6. 发送回复
    let content = RoomMessageEventContent::text_plain(response);
    let _ = room.send(content).await;

    // 7. 停止 typing
    let _ = room.typing_notice(false).await;
}
```

### Sync 后台任务（带重试）

```rust
let client_clone = client.clone();
tokio::spawn(async move {
    const RETRY_DELAYS: &[std::time::Duration] = &[
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(5),
        std::time::Duration::from_secs(30),
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(300),
    ];

    for &delay in RETRY_DELAYS.iter().cycle() {
        match client_clone.sync(SyncSettings::default()).await {
            Ok(()) => break,  // sync 正常结束（通常不会发生）
            Err(e) => {
                tracing::error!("Sync 错误: {e}，{delay:?} 后重试");
                tokio::time::sleep(delay).await;
            }
        }
    }
});
```

## 10. 配置管理

Matrix Bot 需要的配置项：

| 配置 | 说明 | 示例 |
|------|------|------|
| HOMESERVER_URL | Matrix 服务器地址 | `https://matrix.example.org` |
| BOT_USERNAME | Bot 账号 MXID | `@tsukkomi:example.org` |
| BOT_PASSWORD | 密码（或 access token） | 从环境变量读取 |
| STORE_PATH | SQLite 存储路径 | `./data/matrix-store` |

推荐使用环境变量（与现有项目风格一致）：

```rust
let homeserver = std::env::var("MATRIX_HOMESERVER")
    .expect("MATRIX_HOMESERVER 未设置");
let username = std::env::var("MATRIX_USERNAME")
    .expect("MATRIX_USERNAME 未设置");
let password = std::env::var("MATRIX_PASSWORD")
    .expect("MATRIX_PASSWORD 未设置");
```

## 11. Sync v2 的 `SyncRoomMessageEvent` 正确用法（重要）

matrix-sdk 0.18 中，`SyncRoomMessageEvent` 的 API 与旧版本不同：

```rust
// ✅ 推荐：使用 as_original() / as_redacted() 提取内容
if let Some(original) = ev.as_original() {
    let body = &original.content.body;  // 总是可用的 fallback 文本
    let msgtype = &original.content.msgtype;  // m.text, m.emote, m.notice 等
}

// ❌ 旧方法 original_content() 返回 Option<AnySyncMessageLikeEvent>
//    需要额外匹配
```

## 12. 常见操作速查表

| 操作 | 代码 |
|------|------|
| 发送纯文本 | `room.send(RoomMessageEventContent::text_plain("hello")).await?` |
| 发送 Markdown | `room.send(RoomMessageEventContent::text_markdown("**bold**")).await?` |
| 回复消息 | `let reply = Reply::new(&ev); let c = room.make_reply_event(content, reply).await?; room.send(c).await?` |
| 开始 typing | `room.typing_notice(true).await?` |
| 加入房间 | `room.join().await?` |
| 离开房间 | `room.leave().await?` |
| 获取成员列表 | `room.members().await?` |
| 获取发送者 | `ev.sender()` |
| 跳过自己消息 | `ev.sender() == Some(client.user_id().unwrap())` |
| 持久化 session | `serde_json::to_string(&client.session().unwrap())` |
| 恢复 session | `client.restore_session(session).await?` |

## 13. 注意事项与常见坑

1. **Sync 永不返回**：`client.sync()` 会一直运行直到出错或取消。需要在 tokio task 中运行并实现重试逻辑。
2. **StateStore 的重要性**：没有持久化 store，每次重启会全量同步，加密房间会丢失密钥，可能错过消息。
3. **速率限制**：Matrix homeserver 一般有速率限制，发送大量消息可能被暂时封禁（返回 HTTP 429）。
4. **加密房间的兼容性**：E2EE 房间需要 SDK 正确初始化加密引擎。建议开发阶段先用非加密房间测试。
5. **消息格式**：Matrix 的消息使用 `body` + `formatted_body`。纯文本用 `text_plain()`，Markdown 用 `text_markdown()`。
6. **同步性能**：如果 Bot 加入了大量房间（1000+），sync 响应可能很大。可考虑使用 Sliding Sync 或自定义 sync filter。
7. **断线重连**：SDK 内部处理自动重连（HTTP 长轮询超时会自动重发请求）；但 sync 函数返回 `Err` 后需要手动重连。
8. **ruma 宏**：使用 `room_id!()`, `user_id!()` 等宏可以编译时验证 Matrix ID 格式。
9. **Bot 账号 vs Appservice**：简单 Bot 用普通账号即可。Appservice 适合需要管理大量虚拟用户或监听任意房间的场景。
10. **JoinRule**：某些房间需要邀请才能加入，Bot 需要收到邀请事件后才能自动加入。

## 14. 关键文档参考

- [matrix-sdk crate docs](https://docs.rs/matrix-sdk/latest/matrix_sdk/)
- [matrix-rust-sdk GitHub (examples)](https://github.com/matrix-org/matrix-rust-sdk/tree/main/examples)
- [ruma crate docs](https://docs.rs/ruma/latest/ruma/)
- [Matrix Client-Server API 规范](https://spec.matrix.org/latest/client-server-api/)
