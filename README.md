# ClawServer

> **开源、全配置化的 AI Agent 高并发服务端（Rust）**
>
> 基于`axum + tokio + fred` 全异步零锁，单节点可支撑 **10 万级并发**，支持水平扩展，零业务耦合。

---

## ✨ 特性

| 维度 | 设计 |
| --- | --- |
| 并发模型 | `tokio` 多线程事件循环，worker 数 = CPU 核心数 |
| 锁 | 全程 **无 Mutex / RwLock**；配置用 `ArcSwap`，状态下沉 Redis |
| 协议 | HTTP/1.1 + HTTP/2，SSE 流式响应 |
| 状态 | **无状态**，多副本水平扩展 |
| 会话 | `fred` 异步 Redis 池，支持单机 / 集群 / 哨兵 |
| LLM | OpenAI 兼容 HTTP，`reqwest` 连接池 + 流式增量解析 |
| 稳定性 | 令牌桶限流 + 原子熔断器 + 指数退避重试 + 优雅关闭 |
| 可观测 | `tracing` JSON 结构化日志；`/healthz` `/readyz` `/version` |
| 驱动 | 所有任务以 YAML 配置驱动，新增任务零代码 |

---

## 🚀 快速开始

### 1. 依赖
- Rust 1.75+
- Redis 6+（本地或集群）
- 可用的 OpenAI 兼容 LLM 后端

### 2. 编译运行
```bash
export OPENAI_API_KEY=sk-xxx           # 或修改任务 YAML 中 api_key_env
cargo run --release
```

默认监听 `0.0.0.0:8080`，配置目录 `./config`（可通过 `CLAW_CONFIG_DIR` 覆盖）。

### 3. 调用

```bash
curl -N -X POST http://127.0.0.1:8080/v1/agent/stream \
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
data: 为什么...

event: done
data: [DONE]
```

---

## 🧩 任务配置

在 `config/tasks/*.yaml` 内增删任务即可，重启生效：

| task_type | 说明 |
| --- | --- |
| `chat` | 通用多轮对话 |
| `query` | 单轮知识问答 |
| `translate` | 多语言翻译 |
| `headset_interpret` | 耳机实时口语解释 |
| `record_summary` | 会议录音总结 |
| `record_minutes` | 会议纪要生成 |
| `record_todo` | 会议待办项抽取（JSON） |

示例：
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
timeout_secs: 120
```

---

## ⚙️ 配置文件

- `config/config.yaml` 全局：server / redis / llm / rate_limit / circuit_breaker / queue / observability
- `config/tasks/*.yaml` 任务：LLM 参数 / Prompt / 记忆 / 超时

关键可调项（默认即可支撑 10w 并发）：
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

```
                +------------------------------+
  Client  --->  |  Axum (SSE /v1/agent/stream) |
                +---------------+--------------+
                                |  Arc<AgentEngine>
                                v
                +------------------------------+
                |       AgentEngine            |
                |  - TaskRegistry (read-only)  |
                |  - SessionMemory (Redis)     |
                |  - LlmPool (reqwest + CB)    |
                +------+-------------+---------+
                       |             |
                       v             v
                  +---------+   +---------+
                  |  Redis  |   |   LLM   |
                  |  (fred) |   |  HTTP   |
                  +---------+   +---------+
```

所有共享数据 `Arc` 只读，多 tokio worker 间 **零锁竞争**。

---

## 📡 HTTP 接口

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/v1/agent/stream` | SSE 流式 Agent 调用（主入口） |
| GET  | `/healthz` | 进程健康 |
| GET  | `/readyz` | 依赖就绪（Redis、任务数量） |
| GET  | `/version` | 版本号 + 已加载任务 |

### 请求体
```jsonc
{
  "app_id":     "demo",        // 必填
  "user_id":    "u001",        // 必填
  "session_id": "s001",        // 必填
  "task_type":  "chat",        // 必填，必须在 config/tasks/ 已定义
  "content":    "...",         // 必填，<= 512KB
  "model":      "gpt-4o-mini", // 可选，覆盖任务默认 model
  "metadata":   {}             // 可选，业务透传
}
```

### SSE 事件
| event | data | 说明 |
| --- | --- | --- |
| `meta` | `{"request_id":"..."}` | 首个事件，用于链路追踪 |
| `message` | 文本增量 | 多次；按 LLM 推送速率到达 |
| `error` | 错误描述 | 流中异常（不结束 HTTP 200） |
| `done` | `[DONE]` | 流结束 |

---

## 🛡 高并发要点实现索引

| 要求 | 对应实现 |
| --- | --- |
| 全异步非阻塞 | 所有 I/O 为 `tokio` / `reqwest` / `fred` 异步 API |
| 无 Mutex/RwLock | 配置 `ArcSwap`；熔断器 `AtomicU64` CAS；注册表启动期一次构建 |
| 无状态服务 | 会话下沉 Redis，多副本 k8s 水平扩展 |
| 配置只读共享 | `ConfigHandle` + `Arc<AppConfig>`，O(1) 原子加载 |
| SSE 流式 | Axum `Sse` + `tokio_stream`，逐 chunk 转发 |
| LLM 异步 + 池 | `reqwest::Client` 内置连接池 + 按 provider 独立熔断 |
| 限流 / 熔断 / 重试 | `tower_governor` / `util::breaker::CircuitBreaker` / `util::retry::backoff` |
| 10w+ 长连接 | Axum + hyper1；worker = CPU 核数；`max_blocking_threads=512` |
| 低内存 / 高 CPU | 发布 profile 开启 LTO + `codegen-units=1` + `panic=abort` |
| 优雅关闭 | `serve.with_graceful_shutdown` 监听 SIGINT / SIGTERM |
| 健康检查 | `/healthz` `/readyz` `/version` |

---

## 📦 部署

### 本地
```bash
cargo run --release
```

### Docker（示例 Dockerfile）
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
EXPOSE 8080
CMD ["clawserver"]
```

### 水平扩展
- 多副本，无需任何粘性会话（session 下沉 Redis）
- 建议在前面加 L4 负载均衡（Nginx / Envoy / ALB）并开启 HTTP/2 长连接

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
