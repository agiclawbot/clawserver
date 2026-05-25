# ClawServer

> **开源、全配置化的 AI Agent 高并发服务端（Rust）**
>
> 基于 `axum + tokio + fred` 全异步零锁，单节点可支撑 **10 万级并发**，支持水平扩展，零业务耦合。

---

## 特性

| 维度 | 设计 |
|---|---|
| 并发模型 | `tokio` 多线程事件循环，worker 数 = CPU 核心数 |
| 锁 | 全程 **无 Mutex / RwLock**；配置 `Arc<AppConfig>` 只读共享，状态下沉 Redis |
| 协议 | HTTP/1.1，SSE 流式响应 |
| 状态 | **无状态**，多副本水平扩展 |
| 会话 | `fred` 异步 Redis 池，支持单机 / 集群 / 哨兵 |
| LLM | OpenAI 兼容 HTTP，`reqwest` 连接池 + 流式增量解析 |
| 稳定性 | 令牌桶限流 + 原子熔断器 + 指数退避重试 + 优雅关闭 |
| 可观测 | `tracing` JSON 结构化日志；`/metrics` Prometheus；`/healthz` `/readyz` `/version` |
| 驱动 | 所有任务以 YAML 配置驱动，新增任务零代码 |

---

## 快速开始

### 1. 依赖
- Rust 1.75+
- Redis 6+（本地或集群）
- 可用的 OpenAI 兼容 LLM 后端

### 2. 编译运行
```bash
export OPENAI_API_KEY=sk-xxx
cargo run --release
```

默认监听 `0.0.0.0:3385`，配置目录 `./config`（可通过 `CLAW_CONFIG_DIR` 覆盖）。

### 3. 调用

```bash
curl -N -X POST http://127.0.0.1:3385/v1/agent/stream \
  -H 'Content-Type: application/json' \
  -d '{
    "app_id":    "demo",
    "user_id":   "u001",
    "session_id":"s001",
    "task_type": "chat",
    "content":   "给我讲个关于 Rust 的冷笑话"
  }'
```

响应为 SSE 流：
```
event: meta
data: {"request_id":"01HX..."}

event: message
data: 为什么 Rust 程序员总是分不清...

event: done
data: [DONE]
```

---

## 🧩 任务配置

在 `config/tasks/*.yaml` 内增删任务即可，重启生效：

| task_type | 模式 | 场景 |
|---|---|---|
| `chat` | plain | 通用多轮对话 |
| `query` | plain | 单轮知识问答 |
| `translate` | plain | 多语言翻译 |
| `headset_interpret` | plain | 耳机实时口语解释 |
| `record_summary` | plain | 会议录音总结 |
| `record_minutes` | plain | 会议纪要生成 |
| `record_todo` | plain | 会议待办项抽取 |
| `code_review` | **react** | 代码审查（Skill + Tool 联动） |

示例（`plain` 模式：单次 LLM 流式调用）：
```yaml
name: chat
enabled: true
llm:
  provider: openai
  model: gpt-4o-mini
  temperature: 0.7
  max_tokens: 2048
prompt:
  system: |
    你是 ClawServer 的通用对话助手...
  user_template: "{{content}}"
memory:
  enabled: true
  max_turns: 20
mode: plain
timeout_secs: 120
```

示例（`react` 模式：多轮思考→工具调用→观察循环）：
```yaml
name: code_review
enabled: true
mode: react
llm:
  provider: openai
  model: gpt-4o
tools:
  - web_search
  - http_get
skill: code_reviewer
max_iterations: 8
prompt:
  system: "你是一个严谨的代码审查专家。"
  user_template: "请审查以下代码：\n\n{{content}}"
memory:
  enabled: false
  max_turns: 5
```

---

## CLI 调试工具

项目附带独立 CLI 二进制 `clawctl`，**不依赖 Redis**，可用于快速调试：

```bash
# 查看当前配置
cargo run -p claw-cli -- config show

# 直接调用 LLM（测试 prompt/模型/参数）
cargo run -p claw-cli -- llm chat -p openai -m gpt-4o-mini

# Agent 端到端调试（plain/react 模式均支持）
cargo run -p claw-cli -- agent run --task chat --content "你好"
cargo run -p claw-cli -- agent trace --task code_review --content "fn main() {}"

# 内置工具调试
cargo run -p claw-cli -- tool list
cargo run -p claw-cli -- tool invoke time_now '{}'

# 性能基准
cargo run -p claw-cli -- bench tool
```

---

## ⚙️ 配置文件

```
config/
├── config.yaml                  # 全局配置（server/redis/llm/限流/熔断/可观测）
├── tasks/
│   ├── chat.yaml                # 任务定义（按 name 注册）
│   ├── query.yaml
│   └── ...
└── skills/
    └── code_reviewer/
        ├── manifest.yaml        # Skill 元数据（工具白名单 + 默认参数）
        └── instruction.md       # Skill 指令（拼接进 system prompt）
```

关键可调项：
```yaml
server:
  worker_threads: 0          # 0 = CPU 核数
  body_limit_bytes: 1048576
rate_limit:
  per_second: 50000
  burst: 100000
redis:
  pool_size: 64
llm:
  providers:
    openai:
      pool_idle_per_host: 64
```

---

## 🏗 架构

### 分层设计（3 层，6 crate）

```text
┌──────────────────────────────────────────────────────────┐
│                  边界层 (对外暴露)                         │
│  ┌──────────────────┐  ┌──────────────────────────────┐  │
│  │  claw-api        │  │  claw-cli                    │  │
│  │  HTTP/SSE 服务   │  │  命令行调试工具              │  │
│  │  axum Router     │  │  clap 子命令                 │  │
│  │  /v1/agent/stream│  │  agent / llm / tool / config │  │
│  └───────┬──────────┘  └──────────────────────────────┘  │
├──────────┼───────────────────────────────────────────────┤
│          ▼                                                │
│  ┌──────────────────────────────────────────────────┐    │
│  │              编排层 claw-agent                    │    │
│  │  AgentEngine · ReAct 循环 · Session · TaskRegistry│    │
│  │  无锁 · 全 Arc<...> · 运行期只读                  │    │
│  └────────┬─────────────┬──────────────┬─────────────┘    │
├───────────┼─────────────┼──────────────┼──────────────────┤
│           ▼             ▼              ▼                   │
│  ┌──────────┐  ┌────────────┐  ┌────────────────────┐    │
│  │ claw-llm │  │  claw-core │  │  claw-config (重导出)│   │
│  │ LLM 客户端│  │  契约层    │  │                   │    │
│  │ reqwest池 │  │  trait/模型│  │  YAML 加载 + 校验  │    │
│  │ 熔断+重试 │  │  工具/Skill│  │                   │    │
│  └──────────┘  └────────────┘  └────────────────────┘    │
│              基础服务层 + 契约层                           │
└──────────────────────────────────────────────────────────┘
```

### 数据流

```text
POST /v1/agent/stream
     │
     ▼
┌─────────────────┐
│  claw-api        │  DTO 校验 (deny_unknown_fields + 长度限制)
│  agent_stream()  │
└────────┬────────┘
         │ Arc<AgentEngine>
         ▼
┌─────────────────┐
│  claw-agent      │  组合：TaskConfig + Session + LLM
│  run_stream()    │
│                  │
│  ┌─ plain: ─────┤  LLM 一次流式调用 → SSE
│  │              │
│  └─ react: ─────┤  ReAct 多轮循环
│    Thought→Tool │  每轮：LLM → 工具执行 → 观察 → 再思考
│    →Observation │  上限 max_iterations 兜底
└────────┬────────┘
         │ mpsc::Receiver<LlmDelta>
         ▼
┌─────────────────┐
│  SSE 流 (event:  │  message / tool_call / tool_result / thought / done
│  message/done)   │
└─────────────────┘
         │ (后台异步)
         ▼
┌─────────────────┐
│  Redis           │  写入本轮 user_msg + assistant_msg
│  SessionMemory   │  LPUSH + LTRIM + EXPIRE
└─────────────────┘
```

### 两种运行模式

| 模式 | 行为 | 适用场景 |
|------|------|----------|
| `plain` | 单次 LLM 流式调用，零额外开销 | 聊天、翻译、总结 |
| `react` | 多轮 Thought→Tool→Observation 循环 | 代码审查、需要调工具的任务 |

### crate 依赖关系

```
clawserver (根 bin)
  ├── claw-api (HTTP 边界)
  │     └── claw-agent (编排)
  │           ├── claw-llm (LLM 客户端)
  │           ├── claw-core (契约层) ← 最稳定，不依赖任何上层
  │           └── claw-config (重导出)
  ├── claw-cli (CLI 边界)
  │     ├── claw-llm
  │     └── claw-core
  └── claw-config
        └── claw-core (features = ["yaml"])
```

---

## 📡 HTTP 接口

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/v1/agent/stream` | SSE 流式 Agent 调用（主入口） |
| GET  | `/healthz` | 进程健康 |
| GET  | `/readyz` | 依赖就绪（Redis、任务数量） |
| GET  | `/version` | 版本号 + 已加载任务列表 |
| GET  | `/metrics` | Prometheus 文本格式指标 |

### 请求体

```jsonc
{
  "app_id":     "demo",        // 必填，<= 64 字节
  "user_id":    "u001",        // 必填，<= 64 字节
  "session_id": "s001",        // 必填，<= 128 字节
  "task_type":  "chat",        // 必填，<= 64 字节，必须在 config/tasks/ 已定义
  "content":    "...",         // 必填，<= 512KB
  "model":      "gpt-4o-mini", // 可选，覆盖任务默认模型，<= 128 字节
  "metadata":   {}             // 可选，业务透传
}
```

> **注意**：所有字段经过**白名单校验**（`deny_unknown_fields`），传入了未定义的字段会返回 `422 Unprocessable Entity`。

### SSE 事件

| event | data | 说明 |
|---|---|---|
| `meta` | `{"request_id":"..."}` | 首个事件，用于链路追踪 |
| `message` | 文本增量 | 多次；按 LLM 推送速率到达 |
| `tool_call` | `{"name":"...","arguments":...}` | ReAct 模式：LLM 请求调用工具 |
| `tool_result` | `{"name":"...","content":...}` | ReAct 模式：工具返回结果 |
| `thought` | 推理过程 | ReAct 模式：LLM 的思考过程 |
| `error` | 错误描述 | 流中异常（不结束 HTTP 200） |
| `done` | `[DONE]` | 流结束 |

---

## 🛡 高并发要点实现索引

| 要求 | 对应实现 |
|---|---|
| 全异步非阻塞 | 所有 I/O 为 `tokio` / `reqwest` / `fred` 异步 API |
| 无 Mutex/RwLock | 配置 `Arc<AppConfig>`；熔断器 `AtomicU64` CAS；注册表启动期一次构建 |
| 无状态服务 | 会话下沉 Redis，多副本 k8s 水平扩展 |
| 配置只读共享 | `ConfigHandle` + `Arc<AppConfig>`，O(1) 原子加载 |
| SSE 流式 | Axum `Sse` + `tokio_stream`，逐 chunk 转发 |
| LLM 异步 + 池 | `reqwest::Client` 内置连接池 + 按 provider 独立熔断 |
| 限流 / 熔断 / 重试 | `tower_governor` / `util::breaker::CircuitBreaker` / `util::retry::backoff` |
| 10w+ 长连接 | Axum + hyper1；worker = CPU 核数；`max_blocking_threads=512` |
| 低内存 / 高 CPU | 发布 profile 开启 LTO + `codegen-units=1` + `panic=abort` |
| 优雅关闭 | `serve.with_graceful_shutdown` 监听 SIGINT / SIGTERM |
| Prometheus 指标 | `/metrics` 导出请求数 / 耗时 / 并发数直方图 |

---

## 项目结构

```
src/
├── main.rs              # 入口：装配全部依赖 + 启动运行时

crates/
├── claw-core/           # 契约层（trait + 数据模型 + 配置类型 + 工具 + Skill）
│   ├── src/
│   │   ├── config.rs    # AppConfig / TaskConfig / 配置加载
│   │   ├── llm.rs       # ChatProvider trait / LlmDelta / LlmRequest
│   │   ├── chat.rs      # ChatMessage / ChatRole / AssistantToolCall
│   │   ├── tool.rs      # Tool trait / ToolRegistry / ToolCall
│   │   ├── error.rs     # AppError 枚举
│   │   ├── tools/       # 内置工具（TimeNow / HttpGet / WebSearch）
│   │   ├── skill/       # Skill 定义 + 加载 + 注册表
│   │   ├── util/        # 熔断器 / 指数退避
│   │   └── buffer.rs    # 异步 channel 配置
│   └── tests/           # 6 个集成测试文件
│
├── claw-llm/            # LLM 基础服务层
│   ├── src/
│   │   ├── config.rs    # 重导出 claw_core::config
│   │   └── client.rs    # LlmPool + LlmClient (reqwest + SSE 解析)
│   └── tests/pool.rs
│
├── claw-agent/          # 编排层
│   ├── src/
│   │   ├── engine.rs    # AgentEngine（run_stream 入口）
│   │   ├── memory.rs    # SessionStore trait + RedisSessionStore
│   │   ├── react.rs     # ReAct 循环状态机
│   │   └── task.rs      # TaskRegistry（配置索引）
│   └── tests/           # react + task_registry 测试
│
├── claw-api/            # HTTP 边界层
│   ├── src/
│   │   ├── server.rs    # axum Router + 中间件栈 + 优雅关闭
│   │   ├── stream.rs    # SSE 流处理器 /v1/agent/stream
│   │   ├── dto.rs       # AgentRequest DTO + 校验
│   │   └── metrics.rs   # Prometheus 指标 + axum middleware
│   └── tests/           # api + dto 集成测试
│
├── claw-cli/            # CLI 边界层（不依赖 Redis）
│   ├── src/
│   │   ├── cmd/         # 8 个子命令
│   │   ├── builtin.rs   # CLI 内置工具注册
│   │   └── yaml_cfg.rs  # 最小 YAML 加载器
│   └── ...
│
└── claw-config/         # 配置向后兼容重导出层
    ├── src/lib.rs       # pub use claw_core::config::*
    └── tests/yaml.rs
```

---

## 📦 部署

### 本地
```bash
cargo run --release
```

### Docker
```dockerfile
FROM rust:1.79 as builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/clawserver /usr/local/bin/
COPY config /app/config
WORKDIR /app
ENV CLAW_CONFIG_DIR=/app/config
EXPOSE 3385
CMD ["clawserver"]
```

### 水平扩展
- 多副本，无需粘性会话（session 下沉 Redis）
- 建议在前置加 L4 负载均衡（Nginx / Envoy / ALB）并开启 HTTP/2 长连接

---

## 开发

```bash
# 全工作区编译
cargo build --workspace

# 运行全部测试
cargo test --workspace --all-features

# 运行特定 crate 测试
cargo test -p claw-core --all-features
cargo test -p claw-api --all-features
```

### 测试覆盖

| 测试文件 | 覆盖内容 | 数量 |
|---|---|---|
| `claw-api/tests/api.rs` | OPS endpoints / 路由 / DTO 校验 | 8 |
| `claw-api/tests/dto.rs` | AgentRequest 校验规则 | 10 |
| `claw-agent/tests/react.rs` | ReAct 循环（文本/工具/未知工具/迭代上限） | 4 |
| `claw-agent/tests/task_registry.rs` | 任务索引构建 | 2 |
| `claw-core/tests/*` | 工具/Skill/熔断器/重试/注册表/错误类型 | 25 |
| `claw-llm/tests/pool.rs` | LLM 连接池构建 | 4 |
| `claw-config/tests/yaml.rs` | YAML 配置加载与校验 | 5 |
| **合计** | | **68** |

---

## 🔧 系统调优建议（10w 并发长连接）

Linux：
```
# /etc/sysctl.conf
net.core.somaxconn = 65535
net.ipv4.tcp_max_syn_backlog = 65535
net.ipv4.ip_local_port_range = 1024 65535
net.ipv4.tcp_tw_reuse = 1
fs.file-max = 1048576
```
Shell：
```
ulimit -n 1048576
```

---

## 📜 License

Apache-2.0
