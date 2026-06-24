# Rust AI API 封装库对比调研

> 调研 Rust 生态中主流 AI API 客户端/封装库，评估其在 tsukkomi 项目中的适用性。

## 1. 库总览

| 库 | 版本 | 下载量 | stars | 定位 | 协议 |
|---|------|--------|-------|------|------|
| async-openai | 0.41.1 | 5.8M | 1.9k | OpenAI API 封装 | MIT |
| rig-core | 0.39.0 | 1.3M | 3.9k | LLM 应用框架 | MIT |
| genai | 0.6.5 | 239K | 808 | 多 Provider 统一 SDK | MIT/Apache2 |
| langchain-rust | 4.6.0 | 145K | 1.7k | LangChain Rust 移植 | MIT |
| llm | 1.3.8 | 100K | 397 | 多 LLM 后端统一接口 | MIT |
| swiftide | 0.32.1 | 82K | 716 | Agentic RAG/索引 | Apache2 |
| llm_adapter | 0.2.7 | 74K | - | 多 LLM API 适配 | AGPL3 |
| anthropic-sdk | 0.1.5 | 71K | - | Anthropic SDK | MIT |

## 2. 各库详细分析

### 2.1 async-openai

**Stars: 1.9k | Downloads: 5.8M | 版本: 0.41.1**

最成熟的 OpenAI Rust SDK，基于 OpenAI OpenAPI 规范自动生成类型。

**Provider 支持：**
- ✅ OpenAI Chat Completions / Responses API / Assistants
- ⚠️ 通过改 base_url 兼容 DeepSeek、Ollama、vLLM 等
- ✅ 支持 Azure OpenAI

**特点：**
- 类型最完整：覆盖 OpenAI 所有 API（Chat、Responses、Assistants、Audio、Image、Embedding、Fine-tuning 等）
- Builder 模式构造请求
- 内置指数退避重试
- Tower 中间件支持
- WASM 支持
- BYOT（Bring Your Own Types）模式可兼容非标准响应

**缺点：**
- **只针对 OpenAI API 设计**，切换 Provider 需手动改 base_url
- **不支持 Anthropic Messages API**，需 BYOT 模式
- 类型体量巨大，编译时间长
- 每个 Provider 需要自己处理特殊行为

**适用于：** 深度绑定 OpenAI / 需要完整 OpenAI API 覆盖的场景

```rust
use async_openai::{Client, config::OpenAIConfig};

// OpenAI
let client = Client::new();

// DeepSeek（手动改配置）
let config = OpenAIConfig::new()
    .with_base_url("https://api.deepseek.com")
    .with_api_key(env!("DEEPSEEK_API_KEY"));
let client = Client::with_config(config);

let request = CreateChatCompletionRequestArgs::default()
    .model("deepseek-v4-flash")
    .messages(messages)
    .build()?;
let response = client.chat().create(request).await?;
```

---

### 2.2 genai

**Stars: 808 | Downloads: 239K | 版本: 0.6.5**

专为多 Provider 设计的统一 Rust SDK，**推荐用于 tsukkomi**。

**Provider 支持（25+）：**
- ✅ OpenAI (Chat Completions)
- ✅ Anthropic (Messages API)
- ✅ DeepSeek (Chat Completions + Messages API)
- ✅ Ollama (本地部署)
- ✅ Google Gemini
- ✅ AWS Bedrock / Vertex AI
- ✅ Groq / Together / Fireworks
- ✅ GitHub Copilot, xAI, Cohere, Mistral, Perplexity 等

**特点：**
- **一套接口覆盖所有 Provider**：切换 Provider 只需改模型名字符串
- 自动识别 Provider：模型名前缀映射（`gpt-*`→OpenAI, `claude-*`→Anthropic, 其余→Ollama）
- 统一的 `ChatRequest` / `ChatResponse` / `ChatStreamEvent` 类型
- 支持流式、工具调用、图片输入
- `CacheControl` 支持 Anthropic 的提示缓存
- 轻量级，无过多抽象层次

**缺点：**
- 相对较新（0.6.5），API 可能还有变化
- 只支持 Chat 场景（无 Assistants、Responses API 支持）
- 社区规模小于 async-openai

**适用于：** **需要灵活切换 Provider 的项目**（如 tsukkomi）

```rust
use genai::Client;
use genai::chat::{ChatMessage, ChatRequest};

let client = Client::default();

// 切换 Provider 只需改模型名
let providers = &[
    "deepseek:deepseek-v4-flash",
    "ollama:qwen3:14b",
    "claude-haiku-4-5",
    "gpt-5-mini",
];

for model in providers {
    let req = ChatRequest::new(vec![
        ChatMessage::system("你是一个吐槽 bot"),
        ChatMessage::user("今天天气真好"),
    ]);
    let res = client.exec_chat(model, req, None).await?;
    println!("{}: {}", model, res.first_text().unwrap_or(""));
}
```

---

### 2.3 rig-core

**Stars: 3.9k | Downloads: 1.3M | 版本: 0.39.0**

功能最全面的 Rust LLM 应用框架。

**Provider 支持（20+）：**
- ✅ OpenAI / Azure OpenAI
- ✅ Anthropic
- ✅ DeepSeek / Ollama / Groq
- ✅ Cohere / Gemini / Mistral / xAI
- ✅ Hugging Face / Together / Perplexity
- 更多通过 companion crates（Bedrock、Vertex AI 等）

**特点：**
- **Agent 系统**：高级抽象，支持 tool calling、RAG、pipeline
- **Vector store**：内置多种向量数据库支持（MongoDB、Qdrant、SQLite 等）
- **Conversation memory**：对话历史管理
- **Pipeline API**：构建复杂的 AI 工作流
- 完整的 completion/embedding/image/audio 抽象
- 文档最完善，社区最活跃

**缺点：**
- **过度设计**：对于简单的"发消息→收回复"场景，Agent 框架过于沉重
- 学习曲线陡峭
- 编译时间长
- 如果只做简单的 Chat，其抽象层是负收益

**适用于：** 需要 Agent / RAG / 复杂 AI 工作流的项目

```rust
use rig::providers::openai;

let client = openai::Client::from_env()?;
let agent = client.agent(openai::GPT_5_2)
    .preamble("你是一个吐槽 bot")
    .build();

let response = agent.prompt("今天天气真好").await?;
```

---

### 2.4 langchain-rust

**Stars: 1.7k | Downloads: 145K | 版本: 4.6.0**

LangChain 的 Rust 移植版。

**特点：**
- 提供 Chain / Agent / Tool / Memory 等高层抽象
- 支持 OpenAI / Anthropic / Gemini / Ollama 等
- 文档清晰

**缺点：**
- 框架负担重，适合复杂场景
- 项目活跃度一般
- 概念较多（Chain、Document、Memory、Agent 等）

---

### 2.5 llm

**Stars: 397 | Downloads: 100K | 版本: 1.3.8**

轻量的多 LLM 后端统一接口库。

**特点：**
- 支持 OpenAI / Anthropic / Ollama / Gemini 等
- 接口简单，专注于基础的文本生成
- 支持流式、函数调用

**缺点：**
- 功能较少，只覆盖最基础的文本生成
- 社区较小，更新频率低

---

### 2.6 swiftide

**Stars: 716 | Downloads: 82K | 版本: 0.32.1**

面向 Agentic RAG 和索引管道的库。

**特点：**
- Pipeline 式的数据处理
- 内置文档解析（PDF、HTML 等）
- 向量存储集成（Qdrant、Elasticsearch 等）
- 流式索引和查询

**缺点：**
- 偏重 RAG/索引场景，非通用 Chat SDK
- 不适合简单的 Bot 场景

---

### 2.7 llm_adapter

**Stars: - | Downloads: 74K | 版本: 0.2.7**

统一各种 LLM API 的适配层。

**特点：**
- 通过 router/module 模式管理 Provider
- 支持 OpenAI / Anthropic / Ollama 等
- 插件式架构

**缺点：**
- **AGPL-3.0 协议**，商业使用受限
- 文档极少（文档覆盖率 0.28%）
- 版本 0.2.7 还处于早期

---

### 2.8 anthropic-sdk

**Stars: - | Downloads: 71K | 版本: 0.1.5**

Anthropic 的非官方 Rust SDK。

**特点：**
- 专为 Anthropic Messages API 设计
- 支持流式、工具调用

**缺点：**
- 已停止维护（最后更新 2024.07）
- 只支持 Anthropic
- 版本 0.1.5，非常初期

---

## 3. 横向对比

### 3.1 Provider 支持矩阵

| 库 | OpenAI | Anthropic | DeepSeek | Ollama | vLLM | Gemini | 切换成本 |
|---|--------|-----------|----------|--------|------|--------|---------|
| async-openai | ✅原生 | ⚠️ BYOT | ✅改URL | ✅改URL | ✅改URL | ❌ | 手动改配置 |
| **genai** | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | **改字符串** |
| rig-core | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 需选不同 provider |
| llm | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 需选不同 provider |
| langchain-rust | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | 需选不同 provider |

### 3.2 功能矩阵

| 功能 | async-openai | genai | rig-core | llm |
|------|-------------|-------|----------|-----|
| Chat Completions | ✅ | ✅ | ✅ | ✅ |
| Messages API | ⚠️ BYOT | ✅ | ✅ | ❌ |
| Streaming | ✅ | ✅ | ✅ | ✅ |
| Tool Calling | ✅ | ✅ | ✅ | ✅ |
| 图片输入 | ✅ | ✅ | ✅ | ❌ |
| Structured Output | ✅ | ✅ | ✅ | ❌ |
| Agent 框架 | ❌ | ❌ | ✅ | ❌ |
| Vector Store | ❌ | ❌ | ✅ | ❌ |
| Rate Limit Retry | ✅ | ❌ | ❌ | ❌ |
| WASM | ✅ | ❌ | ✅ | ❌ |
| 协议 | MIT | MIT/Apache2 | MIT | MIT |

### 3.3 生态与成熟度

| 维度 | async-openai | genai | rig-core |
|------|-------------|-------|----------|
| 总下载量 | 5.8M | 239K | 1.3M |
| 近30天下载 | 1.95M | 92K | 892K |
| GitHub Stars | 1.9k | 808 | 3.9k |
| 版本数 | 120 | 124 | 59 |
| 最后更新 | 2026.06 | 2026.06 | 2026.06 |
| 文档覆盖率 | 67% | 85% | 45% |

## 4. 对本项目的适用性评估

### 4.1 tsukkomi 的需求

```
需求：
- 发送消息 → 生成吐槽回复
- 支持 DeepSeek（生产）/ Ollama（开发）/ Anthropic（备选）
- 流式输出（可选）
- 轻量，不要过多抽象

不需要：
- Agent 框架
- RAG / Vector Store
- 多模态
- 复杂的工具链
```

### 4.2 各库排名

| 排名 | 库 | 推荐理由 | 过度抽象？ |
|------|-----|---------|-----------|
| 🥇 | **genai** | 轻量统一接口，Provider 切换成本最低 | **否** — 接口简洁 |
| 🥈 | **async-openai** | 最成熟，改 base_url 可兼容 | 否 — 但切换成本高 |
| 🥉 | **rig-core** | 功能最全，但太多不需要的功能 | **是** — Agent 框架 |
| 4 | llm | 简单够用，但生态小 | 否 |
| 5 | langchain-rust | 但框架负担重 | **是** |
| 6 | swiftide | 偏 RAG，不匹配 | **是** |

### 4.3 结论

**genai 最适合 tsukkomi**，理由：

1. **切换 Provider 成本最低**：字符串改模型名，开发环境用 Ollama、生产用 DeepSeek、备选用 Anthropic
2. **接口轻量**：`exec_chat` / `exec_chat_stream` 两个核心方法就够用
3. **Provider 覆盖全**：正好覆盖本项目的三个目标 Provider
4. **无过度抽象**：不像 rig-core 的 Agent 框架，genai 只是 HTTP 客户端封装
5. **Apache2/MIT 双许可**：无 license 风险

**备选方案：async-openai**

如果项目深度绑定 DeepSeek / OpenAI 等 Chat Completions Provider，且不打算切换：
- `async-openai` 更成熟、类型更丰富
- 需要切换到 Ollama / vLLM 时也能通过改 base_url 实现
- 切换到 Anthropic 则需要 BYOT 模式

## 5. 推荐选型

```
tsukkomi
  │
  ├── 🥇 genai（推荐）
  │      Provider 切换：改字符串
  │      开发 → ollama:qwen3:14b
  │      生产 → deepseek:deepseek-v4-flash
  │      备选 → claude-sonnet-4-6
  │
  └── 🥈 async-openai（备选）
         只用一个 Provider 时的成熟选择
```

## 6. 关键文档参考

- [genai crate](https://docs.rs/genai/latest/genai/)
- [async-openai crate](https://docs.rs/async-openai/latest/async_openai/)
- [rig-core crate](https://docs.rs/rig-core/latest/rig_core/)
- [langchain-rust crate](https://crates.io/crates/langchain-rust)
- [swiftide](https://swiftide.rs)
- [llm crate](https://docs.rs/llm/latest/llm/)
- [llm_adapter crate](https://docs.rs/llm_adapter/latest/llm_adapter/)
