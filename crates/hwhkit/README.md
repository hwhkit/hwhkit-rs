# HwhKit

HwhKit 是一个面向 Rust Web 服务的工程化脚手架库，目标是把不变的基础设施沉淀到框架层，让业务项目只保留最小可变部分。

> **0.6.0-alpha.1 — pre-1.0 API stabilization release.** The legacy v1
> surface (`WebServerBuilder`, `JwtAuth`, `hwhkit-macros`,
> `hwhkit-transport`) has been removed. See
> [`MIGRATION.md`](../../MIGRATION.md). Use `hwhkit::run_and_serve` and
> `impl Application for MyApp`. Common imports are at
> `hwhkit::prelude::*`.

当前仓库已升级为 workspace 架构（`0.6.0-alpha.1`），提供：

1. 单一入口：`hwhkit::run_and_serve` + `Application` trait。
2. **OOTB Production Defaults** (Tier 1) — 见下方"Production Readiness (Tier 1)"。
3. **Tier 2 production capabilities** — JWT 链路、限流、幂等键、调度器、熔断器（按需 feature 启用）。
4. CLI：`cargo hwhkit init` / `cargo hwhkit migrate` / `cargo hwhkit dev`。

## Production Readiness (Tier 1)

`run_and_serve(app, bootstrap).await?` 一行调用即获：

| 能力 | 表现 | feature flag (默认开) |
|---|---|---|
| `/health`、`/health/ready` | liveness 永远 200；readiness 并发跑每个 IntegrationProvider 注册的 `HealthCheck`，required 失败→503，optional 失败→200 + `degraded` | `health-endpoints` |
| `/metrics` | Prometheus exporter + tower middleware（`http_requests_total` / `http_request_duration_seconds`）+ `hwhkit_build_info` 标签 | `metrics` |
| `/version` & `/info` | 编译期注入：`git_sha`、`build_time_unix`、`rust_version`、`cargo_version`、已加载集成 | `version-endpoints` |
| Graceful shutdown | SIGTERM/SIGINT → `ShutdownToken`（在 `AppContext` 中）→ `axum::serve.with_graceful_shutdown` + `runtime.shutdown.max_drain_secs` | `graceful-shutdown` |
| Request-ID | tower middleware：读取 `x-request-id` 或生成 UUIDv7，写入 tracing span 并回写响应头 | `request-id` |
| 标准中间件束 | tracing/spans、CORS、gzip+br、timeout、body limit、catch-panic→`application/problem+json`、auth header redact | `middleware-bundle` |
| RFC 7807 错误响应 | `hwhkit::ApiError`（NotFound/Unauthorized/Validation/...）`IntoResponse` 输出 `application/problem+json` | 始终可用 |
| OpenTelemetry OTLP | `hwhkit::observability::otel_layer::init_with_otel(...)`，gRPC 导出，自动注入 service.name/version/environment | `otel`（按需开启） |
| sqlx migrations | `[integrations.sql.postgres.migrations]` (`run_on_start`, `path`) + `cargo hwhkit migrate {create,list,run,revert}` | `migrations` |
| 进程指标 | `process_resident_memory_bytes` / `process_virtual_memory_bytes` / `process_cpu_seconds_total` / `process_open_fds` (Linux/macOS) / `process_threads`，每 5s 采样写入 `/metrics` | `process-metrics`（默认开） |

## Production Readiness (Tier 2, opt-in)

| 能力 | 表现 | feature flag |
|---|---|---|
| JWT 校验链 | `hwhkit::jwt::JwtVerifier` (JWKS 自动拉取 + 缓存、RS256/ES256/HS256/EdDSA 等)、`Claims<T>` axum 抽取器 | `jwt` |
| 限流 (Redis token-bucket) | `RateLimitLayer::per_ip / per_route / per_user`，Lua 原子扣减，命中后 429 + `Retry-After` + RFC 7807 body | `rate-limit` |
| 幂等键 (`Idempotency-Key`) | POST/PUT/PATCH/DELETE 自动重放命中的缓存响应，TTL 默认 24h | `idempotency` |
| 调度器 | 独立 crate `hwhkit-scheduler`：cron + 一次性任务，PG 持久化 (`SELECT FOR UPDATE SKIP LOCKED` 保证多节点互斥) | `scheduler` |
| 熔断器 | `CircuitBreaker` + `CircuitBreakerClient`，closed→open→half-open 三态，可配置失败率/最低请求数/窗口/冷却 | `circuit-breaker` |
| OTel 客户端探针 | sqlx `SqlxSpan::query`、redis `redis_span`、reqwest `tracing_send` — span/属性遵循 OTel 语义约定 | `otel-sqlx` / `otel-redis` / `otel-reqwest` |

## CLI: `cargo hwhkit dev`

读取 `hwhkit.toml`（缺省自动启用 Postgres + Redis）后生成 `target/.hwhkit-dev/docker-compose.yml`，再 shell out 给 `docker compose`：

- `cargo hwhkit dev up [--detach]` — 启动依赖容器
- `cargo hwhkit dev down` — 停止并移除
- `cargo hwhkit dev status` — 查看容器状态
- `cargo hwhkit dev generate --out docker-compose.dev.yml` — 仅生成不启动

## 当前能力（按代码现状）

1. workspace 分层拆包：`hwhkit-config`、`hwhkit-core`、`hwhkit-observability`、`hwhkit-buildinfo`、`hwhkit-scheduler`、`hwhkit-integration-*`、`cargo-hwhkit`。
2. 配置分层加载：`config/default.toml -> config/{env}.toml -> ENV(HWHKIT__) -> remote patch`。
3. 严格校验：配置合法性 + feature/config 一致性校验。
4. 集成 provider：`postgres/redis/mongodb/nats/qdrant/neo4j/s3` —— Handle 内承载真实连接池，并自动暴露 readiness 健康检查。
5. 模板初始化：`minimal-api`、`api-grpc`、`realtime-event`。

## 仓库结构

```text
hwhkit-rs/
  crates/
    hwhkit/                         # facade：对外唯一依赖入口
    hwhkit-config/                  # 配置加载/合并/校验
    hwhkit-core/                    # bootstrap/Application/IntegrationProvider
    hwhkit-observability/           # logging/tracing 初始化
    hwhkit-buildinfo/               # 编译期 git/rustc 信息
    hwhkit-scheduler/               # 调度器（cron + 一次性任务）
    hwhkit-integration-postgres/
    hwhkit-integration-redis/
    hwhkit-integration-mongodb/
    hwhkit-integration-nats/
    hwhkit-integration-qdrant/
    hwhkit-integration-neo4j/
    hwhkit-integration-s3/
    cargo-hwhkit/                   # cargo hwhkit init
  templates/                        # 模板目录
  doc/
```

## Feature 概览（`crates/hwhkit/Cargo.toml`）

默认开启（OOTB production defaults，按需 `default-features = false` 关闭）：

- `health-endpoints`
- `metrics`
- `process-metrics`
- `version-endpoints`
- `middleware-bundle`
- `graceful-shutdown`
- `request-id`

按需启用：

- `jwt`
- 集成：`postgres`、`redis`、`mongodb`、`nats`、`qdrant`、`neo4j`、`s3`
- 观测：`otel`（启用 OTLP gRPC 导出）；`otel-sqlx` / `otel-redis` / `otel-reqwest`（客户端探针）
- 数据库：`migrations`（`postgres` 子能力）
- Tier 2：`rate-limit`（需要 `redis`）、`idempotency`（需要 `redis`）、`circuit-breaker`、`scheduler`

### Minimal preset

The default feature set is lean — health, metrics, request-id,
graceful shutdown, version, middleware bundle, multi-tenant primitives.
For the smallest possible binary set
`default-features = false` and pull in only what you need:

```toml
hwhkit = { version = "0.6.0-alpha.1", default-features = false, features = [
  "graceful-shutdown",
] }
```

聚合：

- `full`

## 安装与使用

### 1) 作为库使用（推荐）

```toml
[dependencies]
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
# Default features already provide health/metrics/version/middleware/shutdown/request-id.
hwhkit = { version = "0.6.0-alpha.1", features = [
  "postgres",
  "redis",
  "mongodb",
  "nats",
  "qdrant",
  "neo4j",
  "s3",
  # Tier 2 (opt-in):
  # "jwt", "rate-limit", "idempotency", "circuit-breaker", "scheduler",
] }
```

### 2) 使用脚手架命令

本仓库内开发：

```bash
cargo run -p cargo-hwhkit -- init my-service --template minimal-api
```

安装为 cargo 子命令后：

```bash
cargo install --path crates/cargo-hwhkit
cargo hwhkit init my-service --template api-grpc
```

## 快速开始 (OOTB)

```rust
use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::prelude::*;
use hwhkit::config::AppConfig;

struct App;

#[async_trait]
impl Application for App {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new().route("/hello", get(|| async { "hi" })))
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Mounts /health, /health/ready, /metrics, /version, /info,
    // applies CORS/timeout/body-limit/compression/catch-panic,
    // adds request-id, listens for SIGTERM/SIGINT for graceful shutdown.
    run_and_serve(App, BootstrapConfig::default()).await
}
```

If you want to manage the listener yourself, use `hwhkit::run` to get a
[`BuiltApplication`] and drive `built.router()` / `built.shutdown()`
yourself.

## 配置示例（v2）

`config/default.toml`：

```toml
[server]
host = "0.0.0.0"
port = 3000

[observability]
service_name = "my-service"
environment = "dev"

[observability.logging]
level = "info"
format = "pretty"

[integrations.sql.postgres]
enabled = true
required = true
url = "postgres://postgres:postgres@127.0.0.1:5432/app"
max_connections = 20

[integrations.redis]
enabled = true
required = false
url = "redis://127.0.0.1:6379"

[integrations.mongodb]
enabled = false
required = false
url = "mongodb://127.0.0.1:27017"
database = "app"

[integrations.messaging.nats]
enabled = false
required = false
url = "nats://127.0.0.1:4222"

[integrations.vector.qdrant]
enabled = false
required = false
url = "http://127.0.0.1:6334"
api_key = ""

[integrations.neo4j]
enabled = false
required = false
url = "bolt://127.0.0.1:7687"
username = "neo4j"
password = "password"

```

## 质量状态

```bash
cargo build  --workspace --all-features
cargo test   --workspace --all-features
cargo clippy --workspace --all-features --all-targets -- -D warnings
```

## 文档

- 执行方案：`doc/2026-02-09-05-execution-plan-confirmed.md`
- 执行进度：`doc/2026-02-09-06-execution-progress.md`
- 架构路线：`doc/2026-02-09-01-architecture-roadmap-v2.md`
- Golden Path：`doc/2026-02-09-02-golden-path-extreme-performance.md`
- 协议网格：`doc/2026-02-09-03-transport-and-protocol-mesh.md`
- 结构方案：`doc/2026-02-09-04-structure-options.md`
- 项目指南：`doc/guides/QUICK_START.md`、`doc/guides/CONTRIBUTING.md`

## 当前边界说明

1. `hwhkit-integration-*` provider 骨架已就位，真实连接池 + readiness 已接入；驱动深度接入会随版本推进。
2. 0.6 周期发布前，`hwhkit-transport` 与 `hwhkit-macros` 这两个旧 crate 已删除（参见 `MIGRATION.md`）。

## License

MIT OR Apache-2.0
