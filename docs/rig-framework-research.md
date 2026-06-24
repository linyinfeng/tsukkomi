# rig 框架完整调研

> rig v0.39.0 | 7.7k stars | MIT | https://github.com/0xPlaygrounds/rig

## 1. 框架全景

`rig` 是一个由 workspace 管理的 Rust LLM 应用框架。顶层 `rig` crate 是门面（facade），通过 feature flags 选择性地引入 companion crates。

### crate 关系

```
rig（门面 crate，re-export rig-core + 可选 companion）
 │
 ├── rig-core        ─ 核心：Agent / Tool / Completion / Embedding
 ├── rig-derive      ─ 过程宏（Tool derive macro）
 ├── rig-memory      ─ 对话记忆策略（sliding window, token budget）
 │
 ├── [Vector Store]
 │   ├── rig-mongodb  ─ MongoDB 向量存储
 │   ├── rig-qdrant   ─ Qdrant 向量存储
 │   ├── rig-sqlite   ─ SQLite (sqlite-vec) 向量存储
 │   ├── rig-lancedb  ─ LanceDB 向量存储
 │   ├── rig-neo4j    ─ Neo4j 向量存储
 │   ├── rig-scylladb ─ ScyllaDB 向量存储
 │   ├── rig-surrealdb─ SurrealDB 向量存储
 │   ├── rig-milvus   ─ Milvus 向量存储
 │   ├── rig-postgres ─ PostgreSQL (pgvector) 向量存储
 │   ├── rig-helixdb  ─ HelixDB 向量存储
 │   ├── rig-s3vectors─ AWS S3 向量存储
 │   └── rig-vectorize─ Cloudflare Vectorize 向量存储
 │
 ├── [Provider]
 │   ├── rig-bedrock  ─ AWS Bedrock 集成
 │   ├── rig-vertexai ─ Google Vertex AI 集成
 │   ├── rig-fastembed─ Fastembed 本地嵌入
 │   └── rig-gemini-grpc ─ Gemini gRPC 集成
 │
 └── [示例] examples/
      ├── agent/                    ─ 最小 Agent
      ├── agent_with_tools/         ─ Tool Calling
      ├── agent_with_memory/        ─ 对话记忆
      ├── agent_stream_chat/        ─ 流式对话
      ├── rag/                      ─ RAG 完整示例
      ├── rag_ollama/              ─ Ollama + RAG
      ├── extractor/               ─ 结构化提取
      ├── multi_turn_agent/        ─ 多轮对话
      ├── multi_agent/             ─ 多 Agent 编排
      ├── agent_autonomous/        ─ 自主 Agent 循环
      ├── agent_orchestrator/      ─ Orchestrator 模式
      ├── agent_evaluator_optimizer/─ Evaluator-Optimizer
      ├── agent_prompt_chaining/   ─ Prompt 链
      ├── agent_routing/           ─ Agent 路由
      ├── calculator_chatbot/      ─ 计算器 Bot
      ├── discord_bot/             ─ Discord Bot
      ├── ... (40+ examples 总计)
```

## 2. Provider 支持

### 内置（rig-core 内）

| Provider | Client 类型 | 环境变量 |
|----------|------------|----------|
| OpenAI | `rig::providers::openai::Client` | `OPENAI_API_KEY` |
| Anthropic | `rig::providers::anthropic::Client` | `ANTHROPIC_API_KEY` |
| DeepSeek | `rig::providers::deepseek::Client` | `DEEPSEEK_API_KEY` |
| Ollama | `rig::providers::ollama::Client` | 无 (默认 localhost) |
| Gemini | `rig::providers::gemini::Client` | `GEMINI_API_KEY` |
| Groq | `rig::providers::groq::Client` | `GROQ_API_KEY` |
| Cohere | `rig::providers::cohere::Client` | `COHERE_API_KEY` |
| Mistral | `rig::providers::mistral::Client` | `MISTRAL_API_KEY` |
| xAI | `rig::providers::xai::Client` | `XAI_API_KEY` |
| Together | `rig::providers::together::Client` | `TOGETHER_API_KEY` |
| Perplexity | `rig::providers::perplexity::Client` | `PERPLEXITY_API_KEY` |
| OpenRouter | `rig::providers::openrouter::Client` | `OPENROUTER_API_KEY` |
| HuggingFace | `rig::providers::huggingface::Client` | `HF_TOKEN` |
| Azure OpenAI | `rig::providers::azure::Client` | `AZURE_OPENAI_*` |
| GitHub Copilot | `rig::providers::copilot::Client` | `GITHUB_TOKEN` |
| Z.ai (智谱) | `rig::providers::zai::Client` | `ZAI_API_KEY` |
| Moonshot (月之暗面) | `rig::providers::moonshot::Client` | `MOONSHOT_API_KEY` |
| MiniMax | `rig::providers::minimax::Client` | `MINIMAX_API_KEY` |
| Xiaomi MiMo | `rig::providers::xiaomimimo::Client` | `MIMO_API_KEY` |
| Llamafile | `rig::providers::llamafile::Client` | 无 |
| Galadriel | `rig::providers::galadriel::Client` | `GALADRIEL_API_KEY` |
| Mira | `rig::providers::mira::Client` | `MIRA_API_KEY` |
| Hyperbolic | `rig::providers::hyperbolic::Client` | `HYPERBOLIC_API_KEY` |

### Companion crate Provider

| Provider | 启用 feature | 说明 |
|----------|-------------|------|
| AWS Bedrock | `bedrock` | rig-bedrock |
| Vertex AI | `vertexai` | rig-vertexai |
| Gemini gRPC | `gemini-grpc` | rig-gemini-grpc |
| Fastembed | `fastembed` | 本地 embedding |

各 Provider 都有独立的 Client 类型，实现了统一的 `CompletionClient` / `EmbeddingsClient` trait。

## 3. 核心能力详解

### 3.1 Agent 系统

Agent 是 rig 的核心抽象，组合了：
- **model** — LLM 模型
- **preamble** — 系统提示词
- **context** — 静态上下文文档
- **tools** — 工具定义
- **dynamic_context** — RAG 动态检索
- **memory** — 对话记忆

```rust
let agent = openai::Client::from_env()?
    .agent(openai::GPT_4O)
    .preamble("你是一个吐槽 bot")
    .context("群聊规则：...")           // 静态上下文
    .tool(GetWeather)                   // 工具
    .dynamic_context(3, index)          // RAG：每次检索 top-3
    .temperature(0.8)
    .build();
```

### 3.2 Tool Calling（自动多轮循环）

Agent 自动处理 tool calling 循环：调用 LLM → 解析 tool call → 执行 → 返回结果 → 继续 LLM 调用。

```rust
// 定义 Tool
#[derive(serde::Deserialize, serde::Serialize)]
struct GetWeather;

impl Tool for GetWeather {
    const NAME: &'static str = "get_weather";

    fn description(&self) -> String {
        "获取天气信息".into()
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "city": {"type": "string"}
            },
            "required": ["city"]
        })
    }

    async fn call(&self, args: serde_json::Value) -> ToolResult {
        let city = args["city"].as_str().unwrap_or("unknown");
        Ok(json!({"temperature": 22, "city": city}).to_string())
    }
}

// Agent 自动处理循环
let response = agent.prompt("北京天气如何？").await?;
```

### 3.3 Conversation Memory（对话记忆）

`rig-memory` crate 提供记忆策略。有两种使用方式：

**方式一：内置 InMemoryConversationMemory**

```rust
use rig::memory::InMemoryConversationMemory;

let agent = openai::Client::from_env()?
    .agent(openai::GPT_4O)
    .preamble("你是助手")
    .memory(InMemoryConversationMemory::default())
    .build();

// 自动管理历史
agent.prompt("我叫张三").await?;
agent.prompt("我叫什么名字？").await?;  // 记得
```

需启用 `memory` feature：

```toml
rig = { version = "0.39", features = ["memory"] }
```

**方式二：手动管理 ConversationHistory**

```rust
use rig::completion::{Message, Role};

let mut history = vec![
    Message { role: Role::Assistant, content: "你好！".into() },
];

// 多轮对话
history.push(Message {
    role: Role::User,
    content: "今天天气真好".into(),
});
let response = agent.chat("今天天气真好", &history).await?;
let text = response.first_sentence().unwrap_or("");
history.push(Message {
    role: Role::Assistant,
    content: text.into(),
});
```

**方式三：rig-memory 的 SlidingWindow / TokenBudget 策略**

```rust
// 使用 rig-memory 策略（需要 rig-memory crate）
// 后续可以配置 sliding window 限制历史长度、token budget 控制总 token 数
```

### 3.4 RAG（检索增强生成）

```rust
use rig::prelude::*;
use rig::vector_store::in_memory_store::InMemoryVectorStore;
use rig::Embed;

// 1. 定义数据结构
#[derive(Embed, Serialize, Clone, Debug)]
struct Knowledge {
    id: String,
    #[embed]
    content: String,
}

// 2. 构建向量存储
let embedding_model = openai_client.embedding_model(openai::TEXT_EMBEDDING_3_SMALL);
let embeddings = EmbeddingsBuilder::new(embedding_model.clone())
    .documents(vec![
        Knowledge { id: "1".into(), content: "群聊规则：禁言期间不可吐槽".into() },
        Knowledge { id: "2".into(), content: "吐槽风格：幽默讽刺，语气友善".into() },
    ])?
    .build()
    .await?;

let vector_store = InMemoryVectorStore::from_documents(embeddings);
let index = vector_store.index(embedding_model);

// 3. RAG Agent：每次 prompt 自动检索 top-2
let rag_agent = openai_client.agent(openai::GPT_4O)
    .preamble("基于上下文回答问题")
    .dynamic_context(2, index)
    .build();

let response = rag_agent.prompt("我可以吐槽吗？").await?;
```

**向量存储选择（feature → crate）：**

| Feature | 后端 | 适用场景 |
|---------|------|----------|
| `rig-core` 内置 | InMemoryVectorStore | 开发/测试，进程内 |
| `mongodb` | MongoDB Atlas | 生产，需已有 MongoDB |
| `qdrant` | Qdrant | 生产，专用向量数据库 |
| `sqlite` | SQLite (sqlite-vec) | 轻量级，单机 |
| `lancedb` | LanceDB | 列式存储，适合本地 |
| `neo4j` | Neo4j | 图数据库场景 |
| `postgres` | PostgreSQL (pgvector) | 已有 Postgres |
| `scylladb` | ScyllaDB | 高可用分布式 |
| `surrealdb` | SurrealDB | 多模型数据库 |
| `milvus` | Milvus | 大规模向量检索 |
| `helixdb` | HelixDB | 边缘/嵌入式 |
| `s3vectors` | AWS S3 | 低成本归档 |

### 3.5 Extractor（结构化输出）

```rust
#[derive(serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
struct Sentiment {
    score: f64,
    label: String,
}

let extractor = openai_client.extractor::<Sentiment>(openai::GPT_4O)
    .preamble("分析以下文本的情感")
    .build();

let ExtractionResponse { extraction, .. } = extractor
    .extract("今天天气真好，心情愉悦")
    .await?;

println!("{}/{}", extraction.label, extraction.score);
```

### 3.6 Pipeline（工作流编排）

rig 提供 pipeline 宏来编排并行/串行工作流：

```rust
use rig::pipeline;

// 并行执行多个操作
let results = pipeline::try_parallel()
    .add(sentiment_analysis)
    .add(keyword_extraction)
    .await?;

// 串行链式
pipeline::chain(step1)
    .chain(step2)
    .chain(step3)
    .await?;
```

### 3.7 Agent 编排模式

rig 的 examples 包含多种 Agent 编排模式的参考实现：

| 模式 | 示例 | 说明 |
|------|------|------|
| Single Agent | `agent/` | 单一 Agent 完成任务 |
| Multi Agent | `multi_agent/` | 多个 Agent 协作 |
| Orchestrator | `agent_orchestrator/` | 主 Agent 分配子任务 |
| Evaluator-Optimizer | `agent_evaluator_optimizer/` | 生成+评估循环 |
| Prompt Chaining | `agent_prompt_chaining/` | 链式处理 |
| Routing | `agent_routing/` | 根据输入路由到不同 Agent |
| Parallelization | `agent_parallelization/` | 并行执行 |
| Autonomous | `agent_autonomous/` | 自主循环迭代 |

### 3.8 其他能力

| 能力 | Feature | 说明 |
|------|---------|------|
| 流式输出 | 内置 | `stream_chat()` → `MultiTurnStreamItem` |
| 图片理解 | 内置 | `ContentPart::from_image_url()` |
| 图片生成 | `image` | DALL-E 等 |
| 音频生成 | `audio` | TTS |
| 嵌入 | 内置 | `embedding_model()` → `Embed` trait |
| 文档加载 | `pdf`, `epub` | PDF/EPUB 解析后作为 context |
| Discord Bot | `discord-bot` | 集成 serenity |
| MCP 工具 | `rmcp` | MCP 协议工具调用 |
| OpenTelemetry | `experimental` | Agent 链路追踪 |
| WASM | `wasm` | WebAssembly 支持 |

## 4. 在 tsukkomi 中的集成方案

### 4.1 初始依赖

```toml
[dependencies]
rig = { version = "0.39", features = ["memory"] }
# 首次使用 InMemory 向量存储，后期可按需切换
# 生产可加 feature = "sqlite" 等

tokio = { version = "*", features = ["macros", "rt-multi-thread"] }
tracing = "*"
tracing-subscriber = "*"
anyhow = "*"
serde = { version = "*", features = ["derive"] }
```

### 4.2 Agent 封装

```rust
use rig::providers::deepseek;
use rig::completion::Prompt;
use rig::memory::InMemoryConversationMemory;

pub struct TsukkomiBot {
    agent: rig::agent::Agent<deepseek::CompletionModel>,
}

impl TsukkomiBot {
    pub fn new() -> Self {
        let agent = deepseek::Client::from_env()
            .expect("DEEPSEEK_API_KEY not set")
            .agent(deepseek::DEEPSEEK_CHAT)
            .preamble("\
                你是一个群聊吐槽役（tsukkomi）bot。\
                要求：简短幽默（20-50字），友善，使用中文网络用语。\
            ")
            .memory(InMemoryConversationMemory::default())
            .build();

        Self { agent }
    }

    pub async fn respond(&self, message: &str) -> Result<String> {
        let response = self.agent.prompt(message).await?;
        Ok(response)
    }
}
```

### 4.3 RAG 能力扩展

```rust
// 当需要"自我学习"时，构建 RAG Agent
use rig::vector_store::in_memory_store::InMemoryVectorStore;
use rig::Embed;

#[derive(Embed, Serialize, Deserialize, Clone, Debug)]
struct LearnedFact {
    id: String,
    #[embed]
    fact: String,       // 被嵌入的字段
    source: String,     // 来源（群名、用户等）
    timestamp: i64,     // 学习时间
}

pub struct LearningBot {
    agent: rig::agent::Agent<deepseek::CompletionModel>,
    store: InMemoryVectorStore<LearnedFact>,
}

impl LearningBot {
    pub async fn learn(&mut self, fact: LearnedFact) -> Result<()> {
        let embedding_model = /* 复用或创建 embedding model */;
        let embeddings = EmbeddingsBuilder::new(embedding_model)
            .documents(vec![fact])?
            .build()
            .await?;
        self.store.add_documents(embeddings);
        Ok(())
    }
}
```

### 4.4 Provider 切换（通过环境变量）

```rust
fn build_agent() -> rig::agent::Agent<impl CompletionModel> {
    let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "deepseek".into());

    match provider.as_str() {
        "deepseek" => {
            let client = deepseek::Client::from_env().unwrap();
            client.agent(deepseek::DEEPSEEK_CHAT)
                .preamble(SYSTEM_PROMPT)
                .build()
        }
        "ollama" => {
            let client = ollama::Client::from_env().unwrap();
            client.agent("qwen3:14b")
                .preamble(SYSTEM_PROMPT)
                .build()
        }
        "anthropic" => {
            let client = anthropic::Client::from_env().unwrap();
            client.agent(anthropic::CLAUDE_SONNET_4_6)
                .preamble(SYSTEM_PROMPT)
                .build()
        }
        _ => panic!("Unknown provider: {provider}"),
    }
}
```

> 注意：由于每个 Provider 的 `CompletionModel` 类型不同，目前 Rust 的类型系统使得在运行时切换 Provider 比较麻烦。一种方案是使用 `Box<dyn CompletionModel>` 或通过 feature flag 在编译时固定 Provider。

## 5. 与 genai 的完整对比

| 维度 | genai | rig |
|------|-------|-----|
| **定位** | API 封装库 | LLM 应用框架 |
| Provider 切换 | 改模型字符串 | 换 Client 类型（编译期） |
| Chat 调用 | `exec_chat(model, req, ..)` | `agent.prompt("...")` |
| Streaming | `ChatStreamEvent` | `MultiTurnStreamItem` |
| Tool Calling | 手动 | **自动多轮循环** |
| RAG | ❌ | ✅ InMemory / MongoDB / Qdrant 等 |
| Memory | ❌ 需手动维护 | ✅ `InMemoryConversationMemory` + rig-memory |
| Structured Output | `ChatResponseFormat` | `extractor` |
| Pipeline | ❌ | ✅ `try_parallel!` / `chain` |
| 多 Agent 编排 | ❌ | ✅ Orchestrator / Router 等 |
| Agent Run 状态机 | ❌ | ✅ `AgentRun` (sans-IO) |
| 内置 Provider | 25+ | 22+ |
| Vector Store | ❌ | 12 种后端 |
| MCP 工具 | ❌ | ✅ rmcp |
| 文档加载 | ❌ | ✅ pdf / epub |
| OpenTelemetry | ❌ | ✅ |
| Discord Bot | ❌ | ✅ |
| 学习成本 | 低 | **中高** |
| 依赖数量 | 19 | 50+ (rig-core) / 按需 (companion crates) |
| 编译速度 | 快 | 较慢 |

## 6. Cargo.toml feature flags 速查

从 `rig` 的 Cargo.toml 可以提取出完整 feature 清单：

```toml
[dependencies]
rig = { version = "0.39", default-features = false, features = [
    # Provider（默认包括 OpenAI）
    "rustls",          # TLS 后端（必选一个）

    # 记忆能力
    "memory",          # Conversation Memory（建议启用）

    # Vector Store（按需选用）
    # "sqlite",        # SQLite 向量存储
    # "mongodb",       # MongoDB
    # "qdrant",        # Qdrant
    # "postgres",      # pgvector

    # 其他能力
    # "derive",        # Tool derive macro
    # "pdf",           # PDF 文档加载
    # "experimental",  # OpenTelemetry
] }
```

## 7. 项目选型结论

```
tsukkomi × rig 框架

当前阶段（MVP）：
  rig-core (Agent + Prompt) + memory feature
  → Agent 封装 LLM 调用
  → InMemoryConversationMemory 管理对话历史
  → 手动管理 RAG（后续扩展）

后续阶段：
  + sqlite/qdrant feature（持久化向量存储）
  + tool calling（搜索等能力）
  + pipeline（复杂工作流）
  + bedrock/vertexai（企业部署）

始终不需要的：
  - discord-bot（不用 Discord）
  - audio / image（无需多媒体生成）
  - epub（不需要）
```

## 8. 关键文档参考

- [rig crate](https://crates.io/crates/rig) / [rig-core docs.rs](https://docs.rs/rig-core/latest/rig_core/)
- [rig GitHub](https://github.com/0xPlaygrounds/rig) — 包含 40+ examples
- [rig examples 目录](https://github.com/0xPlaygrounds/rig/tree/main/examples)
