# HwhKit

HwhKit 是一个面向 Rust Web 服务的工程化脚手架库，目标是把不变的基础设施沉淀到框架层，让业务项目只保留最小可变部分。

> **0.6.0-alpha.1 — pre-1.0 API stabilization release.**
> The legacy v1 surface (`WebServerBuilder`, `WebServer`, `Config`,
> `JwtAuth`, `hwhkit-macros`, `hwhkit-transport`) has been removed.
> See [`MIGRATION.md`](./MIGRATION.md) for the upgrade path and
> [`CHANGELOG.md`](./CHANGELOG.md) for the full diff.
> The recommended entry point is `hwhkit::run_and_serve` driven by
> an `Application` impl; the prelude lives at `hwhkit::prelude::*`.

当前仓库已升级为 workspace 架构（`0.6.0-alpha.1`），提供：

1. 单一 OOTB 入口：`hwhkit::run_and_serve` + `Application` trait。
2. **OOTB Production Defaults (Tier 1)**: `/health`、`/health/ready`、`/metrics`（含进程级 RSS/CPU/FD/线程）、`/version`、`/info`、graceful shutdown、request-id、标准中间件束 — 默认 feature 开启即生效。
3. **Tier 2 production capabilities**：JWT (JWKS+`Claims<T>`)、Redis token-bucket 限流、`Idempotency-Key`、调度器 (`hwhkit-scheduler`)、HTTP 熔断器，按需 feature 启用。
4. CLI：`cargo hwhkit init` 生成项目模板；`cargo hwhkit migrate` 维护 SQL 迁移；`cargo hwhkit dev` 一键起依赖容器（docker compose）。

## Production Readiness (Tier 1, OOTB)

新增的 `hwhkit::run_and_serve(app, bootstrap).await?` 一行调用即可获得：

| Capability | Endpoint / Mechanism | Feature flag (默认开启) |
|---|---|---|
| Liveness / Readiness | `/health` and `/health/ready` (probes every IntegrationProvider concurrently) | `health-endpoints` |
| Prometheus metrics | `/metrics` + HTTP RED middleware (`http_requests_total`, `http_request_duration_seconds`) + `hwhkit_build_info` gauge | `metrics` |
| Build info | `/version`, `/info` (git SHA, build time, rustc 版本，已初始化集成) | `version-endpoints` |
| Graceful shutdown | SIGTERM / SIGINT → `ShutdownToken` (in `AppContext`) → axum `with_graceful_shutdown` + drain timeout | `graceful-shutdown` |
| Request-ID | tower middleware: read `x-request-id` 或生成 UUIDv7，注入 tracing span，回写响应头 | `request-id` |
| Standard middleware bundle | tracing/spans, CORS, gzip+br, body limit, timeout, catch-panic→problem+json, sensitive-header redact | `middleware-bundle` |
| RFC 7807 errors | `hwhkit::ApiError` (`NotFound`/`Unauthorized`/`Validation`/...) → `application/problem+json` | (always available) |
| OpenTelemetry OTLP | `hwhkit::observability::otel_layer::init_with_otel(...)` (gRPC exporter, resource attrs) | `otel` (opt-in) |
| sqlx migrations | `[integrations.sql.postgres.migrations] run_on_start path` + `cargo hwhkit migrate {create,list,run,revert}` | `migrations` |

## 当前能力（按代码现状）

1. workspace 分层拆包：`hwhkit-config`、`hwhkit-core`、`hwhkit-observability`、`hwhkit-buildinfo`、`hwhkit-integration-*`、`hwhkit-scheduler`、`cargo-hwhkit`。
2. 配置分层加载：`config/default.toml -> config/{env}.toml -> ENV(HWHKIT__) -> remote patch`。
3. 严格校验：配置合法性 + feature/config 一致性校验。
4. 集成 provider：`postgres/redis/mongodb/nats/qdrant/neo4j/s3` —— Handle 内承载真实连接池/客户端 (`sqlx::PgPool`、`redis::Client + ConnectionManager`、`async_nats::Client + JetStream`、`qdrant_client::Qdrant`、`mongodb::Client`、`neo4rs::Graph`、`aws_sdk_s3::Client`)，每个 provider 还自动注册 `HealthCheck` 给 readiness 探针使用。Handle 字段为 private，请通过 `handle.pool()` / `handle.client()` 等访问器读取。
5. 模板初始化：`minimal-api`、`api-grpc`、`realtime-event`。

## 仓库结构

```text
hwhkit-rs/
  crates/
    hwhkit/                         # facade：对外唯一依赖入口
    hwhkit-config/                  # 配置加载/合并/校验
    hwhkit-core/                    # bootstrap/Application/IntegrationProvider
    hwhkit-observability/           # logging/tracing 初始化
    hwhkit-integration-postgres/
    hwhkit-integration-redis/
    hwhkit-integration-mongodb/
    hwhkit-integration-nats/
    hwhkit-integration-qdrant/
    hwhkit-integration-neo4j/
    hwhkit-integration-s3/          # AWS S3 / MinIO 兼容存储
    hwhkit-scheduler/               # 持久化后台调度器 (cron + one-shot, PG)
    cargo-hwhkit/                   # cargo hwhkit init / migrate / dev
  templates/                        # 模板目录
  doc/
```

## Feature 概览（`crates/hwhkit/Cargo.toml`）

可选能力：

- `jwt` — JWT 验证链 (JWKS / HMAC, axum extractor)
- `multi-tenant` — `TenantId` / `TenantScope` / 抽取层
- `rate-limit`, `idempotency`, `circuit-breaker` — Tier-2 中间件
- `scheduler` — 持久化后台调度器
- `otel`, `otel-sqlx`, `otel-redis`, `otel-reqwest` — OpenTelemetry

集成：

- `postgres`, `redis`, `mongodb`, `nats`, `qdrant`, `neo4j`, `s3`

聚合：

- `full`

## 最小服务 (`hwhkit::prelude`)

```rust
use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::prelude::*;

struct DemoApp;

#[async_trait]
impl Application for DemoApp {
    async fn build_router(
        &self,
        _ctx: AppContext,
        _cfg: &hwhkit::config::AppConfig,
    ) -> Result<Router> {
        Ok(Router::new().route("/", get(|| async { "ok" })))
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_and_serve(DemoApp, BootstrapConfig::default()).await
}
```

## 安装与使用

### 1) 作为库使用（推荐）

```toml
[dependencies]
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
axum = "0.7"
hwhkit = { version = "0.6.0-alpha.1", features = [
  "postgres",
  "redis",
  "scheduler",
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

## 快速开始

```rust
use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::prelude::*;
use hwhkit::config::AppConfig;

struct App;

#[async_trait]
impl Application for App {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new().route("/healthz", get(|| async { "ok" })))
    }
}

#[tokio::main]
async fn main() {
    // One call: bootstrap, mount /health//metrics//version, install
    // SIGINT/SIGTERM, serve until drain.
    run_and_serve(App, BootstrapConfig::default())
        .await
        .expect("server failed");
}
```

If you want to manage the listener yourself, use `hwhkit::run` instead
to receive a [`BuiltApplication`] you can drive via
`built.router()` / `built.shutdown()`.

## 配置示例

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
