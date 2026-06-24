# genai 库使用指南 — tsukkomi 吐槽 Bot

> genai v0.6.5 — 25+ AI Provider 统一 Rust SDK
> 仓库: https://github.com/jeremychone/rust-genai

## 1. 概述

`genai` 是一个多 Provider 统一的 Rust LLM SDK，通过一套接口支持 OpenAI Chat Completions、Anthropic Messages API、Google Gemini、Ollama 等 25+ Provider。

### 核心设计理念

```
统一 Chat API → 适配不同 Provider 的底层 API 规范
     │
     ├── openai:gpt-5-mini        → Chat Completions
     ├── anthropic:claude-sonnet  → Messages API
     ├── deepseek:deepseek-v4     → Chat Completions
     ├── ollama:qwen3             → Chat Completions
     └── gemini:gemini-3-flash    → Gemini SDK
```

Provider 切换只需改**模型名字符串**，无需改代码逻辑。

### Provider 识别规则

genai 通过模型名前缀自动识别 Provider（AdapterKind）：

| 模型名前缀 | AdapterKind | 底层 API |
|-----------|------------|----------|
| `gpt-*` | OpenAI | Chat Completions |
| `claude-*` | Anthropic | Messages API |
| `gemini-*` | Gemini | Gemini SDK |
| `deepseek-*` | DeepSeek | Chat Completions |
| `command-*` | Cohere | Cohere SDK |
| `glm-*` | ZAI (智谱) | Chat Completions |
| `o1-*`, `o3-*` | OpenAI | Chat Completions |
| 其他 | **Ollama** (默认 fallback) | Chat Completions |

也可显式指定命名空间：`ollama:qwen3`, `groq::llama-3`, `github_copilot::openai/gpt-4`。

## 2. 安装与初始化

### Cargo.toml

```toml
[dependencies]
genai = "0.6"
tokio = { version = "*", features = ["macros", "rt-multi-thread"] }
tracing-subscriber = "*"
anyhow = "*"
```

### 环境变量

genai 默认从环境变量读取 API key：

| Provider | 环境变量 |
|----------|----------|
| OpenAI | `OPENAI_API_KEY` |
| Anthropic | `ANTHROPIC_API_KEY` |
| DeepSeek | `DEEPSEEK_API_KEY` |
| Gemini | `GEMINI_API_KEY` |
| Ollama | 无需 key（本地） |
| Groq | `GROQ_API_KEY` |
| AI/慧 perc | `ZAI_API_KEY` / `ALIYUN_API_KEY` |

## 3. 基础使用

### 3.1 创建 Client

```rust
use genai::Client;

// 默认 client（从环境变量读 key）
let client = Client::default();

// 或 builder 模式
let client = Client::builder()
    .with_config(
        ClientConfig::default()
            .with_chat_options(ChatOptions::default().with_temperature(0.7))
    )
    .build();
```

### 3.2 简单对话（非流式）

```rust
use genai::chat::{ChatMessage, ChatRequest};

let chat_req = ChatRequest::new(vec![
    ChatMessage::system("你是一个群聊吐槽 bot，回复简短幽默"),
    ChatMessage::user("今天天气真好"),
]);

// 非流式调用
let chat_res = client.exec_chat(
    "deepseek:deepseek-v4-flash",  // 模型名
    chat_req,
    None,                          // 可选 ChatOptions
).await?;

// 获取回复文本
let text = chat_res.first_text().unwrap_or("");
println!("{text}");

// 查看用量
println!("{:?}", chat_res.usage);
```

### 3.3 流式对话

```rust
use genai::chat::printer::print_chat_stream;
use futures::StreamExt;

let chat_req = ChatRequest::new(vec![
    ChatMessage::system("你是一个吐槽 bot"),
    ChatMessage::user("今天天气真好"),
]);

// 流式调用
let chat_stream = client.exec_chat_stream(
    "deepseek:deepseek-v4-flash",
    chat_req,
    None,
).await?;

// 方式一：使用内置 printer
let full_text = print_chat_stream(chat_stream, None).await?;

// 方式二：手动处理 stream events
let mut stream = client.exec_chat_stream(MODEL, chat_req, None).await?;
while let Some(event) = stream.next().await {
    match event {
        ChatStreamEvent::Chunk(chunk) => {
            print!("{}", chunk.content);
        }
        ChatStreamEvent::End(end) => {
            println!("\n--- 完成 ---");
            if let Some(usage) = end.usage {
                println!("用量: {:?}", usage);
            }
        }
        _ => {}
    }
}
```

### 3.4 多轮对话（Conversation）

```rust
let client = Client::default();
let mut chat_req = ChatRequest::default()
    .with_system("Answer in one sentence");

let questions = ["Why is the sky blue?", "Why is it red sometimes?"];

for &question in &questions {
    chat_req = chat_req.append_message(ChatMessage::user(question));

    let chat_res = client.exec_chat_stream(
        "deepseek:deepseek-v4-flash",
        chat_req.clone(),
        None,
    ).await?;

    let assistant_answer = print_chat_stream(chat_res, None).await?;

    // 将 assistant 回复加入历史
    chat_req = chat_req.append_message(ChatMessage::assistant(assistant_answer));
}
```

## 4. 核心 API

### `ChatRequest`

```rust
// 构造方式
ChatRequest::new(vec![messages...])
ChatRequest::default().with_system("system prompt")
ChatRequest::default().append_message(msg)

// 添加消息
req.append_message(ChatMessage::user("text"))
req.append_message(ChatMessage::assistant("text"))
req.append_message(ChatMessage::system("text"))
req.append_message(tool_calls)     // Vec<ToolCall>
req.append_message(tool_response)  // ToolResponse

// 配置工具
req.with_tools(vec![tool1, tool2])

// 克隆用于多轮对话
req.clone()
```

### `ChatMessage`

```rust
// 基础消息
ChatMessage::user("用户消息")
ChatMessage::assistant("助手回复")
ChatMessage::system("系统提示")

// 带 options 的消息（用于缓存控制）
ChatMessage::system("长文本系统提示")
    .with_options(CacheControl::Ephemeral5m)  // Anthropic 5min 缓存
    .with_options(CacheControl::Ephemeral1h)  // Anthropic 1h 缓存

// 多模态（图片）
ChatMessage::user(vec![
    ContentPart::from_text("描述这张图"),
    ContentPart::from_image_image_url("https://example.com/photo.jpg"),
])
```

### `ChatOptions`

```rust
ChatOptions::default()
    .with_temperature(0.8)     // 控制随机性
    .with_top_p(0.95)          // nucleus sampling
    .with_max_tokens(200u32)   // 最大输出 token 数
    .with_min_tokens(10u32)    // 最小输出 token 数
    .with_stop_sequences(vec!["\n".into()])
    .with_seed(42u32)          // 固定随机种子
    .with_cache_control(true)  // 启用 Anthropic 缓存
    .with_reasoning_effort(ReasoningEffort::Low)  // o-series thinking
    .with_service_tier(ServiceTier::Auto)
```

### `ChatResponse`

```rust
let res = client.exec_chat(MODEL, req, None).await?;

res.first_text()                          // Option<&str> 第一条文本
res.content.text()                        // Vec<&str> 所有文本
res.content.first_text()                  // Option<&str>
res.content.first_text_content()          // Option<&TextContent>
res.into_tool_calls()                     // Vec<ToolCall>
res.usage.prompt_tokens                   // Option<i32>
res.usage.completion_tokens               // Option<i32>
res.usage.total_tokens                    // Option<i32>
```

### 工具/函数调用

```rust
use genai::chat::{Tool, ToolResponse};
use serde_json::json;

// 1. 定义工具
let weather_tool = Tool::new("get_weather")
    .with_description("获取天气信息")
    .with_schema(json!({
        "type": "object",
        "properties": {
            "city": {"type": "string"},
            "unit": {"type": "string", "enum": ["C", "F"]}
        },
        "required": ["city"]
    }));

// 2. 发起请求
let req = ChatRequest::new(vec![
    ChatMessage::user("Tokyo 天气如何？"),
]).with_tools(vec![weather_tool]);

let res = client.exec_chat(MODEL, req.clone(), None).await?;

// 3. 提取工具调用
let tool_calls = res.into_tool_calls();
for tc in &tool_calls {
    println!("call_id: {}", tc.call_id);
    println!("fn_name: {}", tc.fn_name);
    println!("args: {}", tc.fn_arguments);
}

// 4. 返回工具结果
let tool_response = ToolResponse::new(
    tool_calls[0].call_id.clone(),
    json!({"temperature": 22}).to_string(),
);

// 5. 继续对话
let req = req.append_message(tool_calls)
    .append_message(tool_response);
let final_res = client.exec_chat_stream(MODEL, req, None).await?;
```

## 5. Provider 切换实战

### 5.1 统一接口切换

```rust
async fn chat_with_any_provider(
    client: &Client,
    model: &str,
    msg: &str,
) -> Result<String> {
    let req = ChatRequest::new(vec![
        ChatMessage::system("你是一个吐槽 bot"),
        ChatMessage::user(msg),
    ]);
    let res = client.exec_chat(model, req, None).await?;
    Ok(res.first_text().unwrap_or("").to_string())
}

// 切换 provider 只需改模型名
chat_with_any_provider(&client, "deepseek:deepseek-v4-flash", "你好").await?;
chat_with_any_provider(&client, "ollama:qwen3", "你好").await?;
chat_with_any_provider(&client, "claude-haiku-4-5", "Hello").await?;
chat_with_any_provider(&client, "gpt-5-mini", "Hello").await?;
```

### 5.2 DeepSeek（生产推荐）

```rust
// 环境变量 DEEPSEEK_API_KEY
let res = client.exec_chat(
    "deepseek:deepseek-v4-flash",
    ChatRequest::new(vec![ChatMessage::user("吐槽一下")]),
    None,
).await?;
```

### 5.3 Ollama（本地开发）

```rust
// 无需环境变量
let res = client.exec_chat(
    "ollama:qwen3:14b",  // 或直接 "qwen3:14b"（默认 fallback 到 Ollama）
    ChatRequest::new(vec![ChatMessage::user("吐槽一下")]),
    None,
).await?;
```

### 5.4 Anthropic（备用通道）

```rust
// 环境变量 ANTHROPIC_API_KEY
// 可使用 Anthropic 的缓存功能
let req = ChatRequest::default()
    .append_message(
        ChatMessage::system("你是一个吐槽 bot".repeat(100))  // 确保超过缓存阈值
            .with_options(CacheControl::Ephemeral5m)   // 5分钟缓存
    )
    .append_message(ChatMessage::user("吐槽一下"));

let res = client.exec_chat("claude-sonnet-4-6", req, None).await?;
```

## 6. 缓存控制（Anthropic Messages API）

genai 通过 `CacheControl` 枚举支持 Anthropic 的提示缓存：

```rust
use genai::chat::CacheControl;

// 每个消息独立控制
ChatMessage::system("长文本...")
    .with_options(CacheControl::Ephemeral5m)  // 5min 缓存（默认）
    .with_options(CacheControl::Ephemeral1h)  // 1h 缓存

// 全局启用（自动在 system 消息上标记 cache_control）
let options = ChatOptions::default()
    .with_cache_control(true);
```

缓存命中后的用量信息：

```rust
let usage = res.usage;
if let Some(details) = &usage.prompt_tokens_details {
    println!("缓存创建 tokens: {:?}", details.cache_creation_tokens);
    println!("缓存命中 tokens: {:?}", details.cached_tokens);
    if let Some(cc) = &details.cache_creation_details {
        println!("1h 缓存写入: {:?}", cc.ephemeral_1h_tokens);
        println!("5m 缓存写入: {:?}", cc.ephemeral_5m_tokens);
    }
}
```

## 7. 与 async-openai 对比

| 维度 | genai | async-openai |
|------|-------|-------------|
| Provider 支持 | 25+ | 仅 OpenAI（可改 base_url） |
| 统一接口 | ✅ 同一 Chat API | ❌ 不同 provider 需不同 client |
| Rust 类型体量 | 轻量（统一的 Chat 类型） | 庞大（各 API 独立类型） |
| 版本 | 0.6.5 | 0.41.1 |
| 成熟度 | 较新 | 更成熟 |
| 多模态 | ✅ ContentPart 统一 | ✅ 细粒度类型 |
| Messages API | ✅ 原生支持 Anthropic | ⚠️ BYOT 模式 |
| Stream | ✅ `ChatStreamEvent` 枚举 | ✅ `ChatCompletionStreamResponse` |

**选择建议**：
- 需要多 Provider 灵活切换 → **genai**
- 深度绑定 OpenAI（Responses API、Assistants API） → async-openai
- 本项目（tsukkomi） → **genai**（灵活切换 DeepSeek/Ollama/Anthropic）

## 8. 在 tsukkomi 中的集成方案

### Cargo.toml

```toml
[dependencies]
genai = "0.6"
tokio = { version = "*", features = ["macros", "rt-multi-thread"] }
tracing = "*"
tracing-subscriber = "*"
anyhow = "*"
serde = { version = "*", features = ["derive"] }
```

### 抽象层设计

```rust
use genai::Client as GenaiClient;
use genai::chat::*;

pub struct LlmService {
    client: GenaiClient,
    model: String,  // "deepseek:deepseek-v4-flash" 等
}

impl LlmService {
    pub fn new(provider: &str, model: &str) -> Self {
        let model_name = format!("{provider}:{model}");
        Self {
            client: GenaiClient::default(),
            model: model_name,
        }
    }

    pub async fn generate_tsukkomi(
        &self,
        message: &str,
        history: &[ChatMessage],
    ) -> Result<String> {
        let mut req = ChatRequest::default()
            .with_system(SYSTEM_PROMPT);

        for msg in history {
            req = req.append_message(msg.clone());
        }
        req = req.append_message(ChatMessage::user(message));

        let res = self.client.exec_chat(&self.model, req, None).await?;
        Ok(res.first_text().unwrap_or("").to_string())
    }

    pub async fn generate_tsukkomi_stream(
        &self,
        message: &str,
        history: &[ChatMessage],
    ) -> Result<impl futures::Stream<Item = String>> {
        let mut req = ChatRequest::default()
            .with_system(SYSTEM_PROMPT);

        for msg in history {
            req = req.append_message(msg.clone());
        }
        req = req.append_message(ChatMessage::user(message));

        let stream = self.client.exec_chat_stream(&self.model, req, None).await?;
        Ok(stream.map(|event| match event {
            ChatStreamEvent::Chunk(c) => c.content,
            _ => String::new(),
        }))
    }
}

// 使用示例
let service = LlmService::new("deepseek", "deepseek-v4-flash");
// 或本地开发
let service = LlmService::new("ollama", "qwen3:14b");
// 或备用
let service = LlmService::new("anthropic", "claude-sonnet-4-6");
```

### 配置管理

```rust
#[derive(Clone, Deserialize)]
pub struct LlmConfig {
    pub provider: String,    // "deepseek" / "ollama" / "anthropic"
    pub model: String,       // "deepseek-v4-flash" / "qwen3:14b" / "claude-sonnet-4-6"
    pub temperature: f64,
    pub max_tokens: u32,
}

impl LlmConfig {
    pub fn from_env() -> Self {
        Self {
            provider: std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "ollama".into()),
            model: std::env::var("LLM_MODEL").unwrap_or_else(|_| "qwen3:14b".into()),
            temperature: std::env::var("LLM_TEMPERATURE")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(0.8),
            max_tokens: std::env::var("LLM_MAX_TOKENS")
                .ok().and_then(|v| v.parse().ok()).unwrap_or(200),
        }
    }

    pub fn to_model_name(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }
}
```

## 9. 常用示例汇总

| 场景 | 代码 |
|------|------|
| 创建 Client | `Client::default()` |
| 非流式对话 | `client.exec_chat(model, req, None).await?` |
| 流式对话 | `client.exec_chat_stream(model, req, None).await?` |
| 添加系统提示 | `ChatRequest::default().with_system("...")` |
| 多轮对话 | `req.append_message(ChatMessage::assistant(text))` |
| 工具调用 | `req.with_tools(vec![tool])` + `res.into_tool_calls()` |
| 图片输入 | `ContentPart::from_image_url("https://...")` |
| 缓存控制 | `ChatMessage::system("...").with_options(CacheControl::Ephemeral5m)` |
| 用量信息 | `res.usage.prompt_tokens` |
| 切换 DeepSeek | `"deepseek:deepseek-v4-flash"` |
| 切换 Ollama | `"ollama:qwen3:14b"` |
| 切换 Anthropic | `"claude-sonnet-4-6"` |

## 10. 关键文档参考

- [genai crate docs](https://docs.rs/genai/latest/genai/)
- [genai GitHub (examples)](https://github.com/jeremychone/rust-genai/tree/main/examples)
- [genai 作者博客/视频](https://github.com/jeremychone/rust-genai)
