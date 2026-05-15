# HwhKit 快速开始

5 分钟从零到一个生产级 Rust Web 服务。基于 **0.6.0-alpha.1**。

如果你看到过去版本提到 `WebServerBuilder` / `hwhkit::Config` —— 那是
v1 API，0.6 已删除。本文档是当前版本的唯一权威入门。

---

## 1. Hello World（30 秒）

```bash
cargo new my-service
cd my-service
```

`Cargo.toml`:

```toml
[package]
name = "my-service"
version = "0.1.0"
edition = "2021"

[dependencies]
hwhkit = "0.6.0-alpha.1"
axum = "0.7"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
```

`src/main.rs`:

```rust
use async_trait::async_trait;
use axum::{routing::get, Router};
use hwhkit::config::{AppConfig, BootstrapConfig};
use hwhkit::{run_and_serve, AppContext, Application};

/// The only mandatory trait. Return the user-routed `axum::Router`;
/// hwhkit mounts `/health`, `/metrics`, `/version`, request-id and
/// graceful shutdown around it OOTB.
struct MyApp;

#[async_trait]
impl Application for MyApp {
    async fn build_router(
        &self,
        _ctx: AppContext,
        _cfg: &AppConfig,
    ) -> hwhkit::Result<Router> {
        Ok(Router::new().route("/", get(|| async { "hello from hwhkit" })))
    }
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_and_serve(MyApp, BootstrapConfig::default()).await
}
```

`cargo run` → 启动在 `0.0.0.0:3000`。**不需要写 config 文件** —— 0.6
之后 `config/default.toml` 是可选的，缺省值适用于本地开发。

试试：

```bash
curl localhost:3000/              # → hello from hwhkit
curl localhost:3000/health        # → {"status":"ok"}
curl localhost:3000/health/ready  # → {"status":"up","degraded":false,...}
curl localhost:3000/metrics       # → Prometheus 文本
curl localhost:3000/version       # → 构建信息
curl localhost:3000/info          # → 服务名 / 环境 / 已初始化集成
```

`/health` `/metrics` `/version` `/info`、request-id 中间件、CORS、压缩、panic→problem+json、SIGTERM graceful shutdown 全部 **默认开启**。这就是 hwhkit 帮你免费拿到的东西。

---

## 2. 配置文件（什么时候需要）

不需要时 hwhkit 用 `AppConfig::default()`（host=`0.0.0.0`, port=3000,
service_name="hwhkit-service"）。改这些字段就建一个 `config/default.toml`：

```toml
[server]
host = "0.0.0.0"
port = 8080

[observability]
service_name = "my-service"
environment = "dev"
```

环境覆盖：`config/{dev,test,prod}.toml`，根据 `HWHKIT__ENVIRONMENT`
环境变量或 `BootstrapConfig::with_environment()` 选择。

环境变量覆盖：`HWHKIT__SERVER__PORT=9000` 覆盖 `server.port`（双下划线
= 嵌套分隔符）。

---

## 3. 加 Postgres（2 分钟）

打开 `postgres` feature：

```toml
[dependencies]
hwhkit = { version = "0.6.0-alpha.1", features = ["postgres"] }
sqlx = { version = "0.8", default-features = false, features = ["runtime-tokio-rustls", "postgres"] }
```

`config/default.toml`：

```toml
[integrations.sql.postgres]
enabled = true
required = true                  # bootstrap 失败时 panic
url = "postgres://user:pw@localhost:5432/mydb"
max_connections = 20

# 可选 —— 任意未列字段都用安全默认（5s/500ms/5s）
[integrations.sql.postgres.resilience]
connect_timeout_ms = 5000
op_timeout_ms      = 5000
probe_timeout_ms   = 500
shutdown_timeout_ms = 5000
```

handler 里拿 pool：

```rust
use hwhkit::postgres::PostgresHandle;

async fn build_router(
    &self,
    ctx: AppContext,
    _cfg: &AppConfig,
) -> hwhkit::Result<Router> {
    let pg = ctx.get::<PostgresHandle>().expect("postgres enabled");
    let pool = pg.pool().clone();

    Ok(Router::new()
        .route("/db/now", get(move || {
            let pool = pool.clone();
            async move {
                let (now,): (String,) = sqlx::query_as("SELECT now()::text")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                now
            }
        })))
}
```

**就这么简单**。bootstrap 期间 `PostgresProvider` 已经：
- 用 `connect_timeout` 做 bounded 连接
- 运行 `SELECT 1` 冒烟测试
- 把 `PostgresHandle` 注入 `AppContext`
- 注册 readiness probe，并发挂到 `/health/ready`
- 注册 graceful shutdown hook（`shutdown_timeout` 内 `PgPool::close()`）
- 后台 spawn metrics sampler，每 10s 发 `postgres_pool_size` / `postgres_pool_idle` gauge

---

## 4. 集成清单

每个集成是一个 feature flag + 一个 config section + 从 `AppContext` 拿
出来的 Handle 类型。

| Feature | Config section | Handle 类型 |
|---|---|---|
| `postgres` | `[integrations.sql.postgres]` | `hwhkit::postgres::PostgresHandle` |
| `redis` | `[integrations.redis]` | `hwhkit::redis::RedisHandle` |
| `mongodb` | `[integrations.mongodb]` | `hwhkit::mongodb::MongoDbHandle` |
| `nats` | `[integrations.messaging.nats]` | `hwhkit::nats::NatsHandle` |
| `qdrant` | `[integrations.vector.qdrant]` | `hwhkit::qdrant::QdrantHandle` |
| `neo4j` | `[integrations.neo4j]` | `hwhkit::neo4j::Neo4jHandle` |
| `s3` | `[integrations.storage.s3]` | `hwhkit::s3::S3Handle` |

所有 Handle 都有：
- 内部 SDK 客户端的 accessor（`pool()` / `client()` / `manager()` …）
- `op_timeout() -> Duration` —— 你应该用它包裹长查询：
  ```rust
  tokio::time::timeout(handle.op_timeout(), my_query).await??
  ```

---

## 5. 生产能力（feature 一开就有）

| Feature | 提供什么 |
|---|---|
| `jwt` | JWKS 自动拉取 + axum 提取器 `Claims<T>` |
| `rate-limit` | Redis token-bucket 限流（需 `redis`） |
| `idempotency` | `Idempotency-Key` 头幂等性（需 `redis`） |
| `circuit-breaker` | 出站 HTTP 熔断器（reqwest） |
| `scheduler` | 持久化 cron + one-shot 调度器（PG-backed） |
| `multi-tenant` | `TenantId` / `TenantScope` 原语（默认开） |
| `otel` | OpenTelemetry OTLP 导出 |
| `otel-sqlx` / `otel-redis` / `otel-reqwest` | 跨边界 trace 透传 |

`full` feature 一次性开所有 integration + 所有 Tier-2 能力，适合
"我先把架子搭起来再裁" 的场景。

---

## 6. 高级入口（你需要更精细控制时）

- `hwhkit::run(app, bootstrap)` —— 只 bootstrap，返回 `BuiltApplication`。
  你自己驱动 axum serve（多 listener / HTTPS / 自定义 shutdown 顺序）。
- `hwhkit::production::server::run_with_listener(built, listener)` ——
  接管一个预绑定的 `TcpListener`。这是 e2e 测试 + systemd socket
  activation 的入口。

---

## 7. 在哪查更多

| 想看什么 | 去哪 |
|---|---|
| 完整 examples | `examples/` 目录 —— `minimal`、`postgres-rest`、`full-stack` |
| 配置 schema 字段说明 | `hwhkit_config::AppConfig` 的 rustdoc |
| 中间件 / 生产端点细节 | `hwhkit::production::*` 的 rustdoc |
| 集成韧性设计 | `doc/2026-05-14-01-integration-resilience-audit.md` |
| live 测试怎么跑 | `TESTING.md` |
| 发布工具 | `Makefile` 的 `make help` |

---

## 8. 常见坑

- **`failed to load configuration`** —— 0.6 之前 `default.toml` 是必需的，
  现在可选。如果你在 0.5 → 0.6 升级时撞到这个错，删掉旧的报错代码即可。
- **`integration ... is required but missing url`** —— 你打开了 feature
  并把 `required = true`，但 url 是空。要么填 url，要么 `required = false`。
- **handler 拿不到 `PostgresHandle`** —— `ctx.get::<PostgresHandle>()`
  返回 `Option<Arc<T>>`。返回 `None` 几乎只可能是该 feature 没开，或
  `enabled = false`。检查 `BuiltApplication::initialized_integrations()`
  确认它确实初始化了。
- **`/health/ready` 一直 503** —— 看 response body 里的 `checks` 数组，
  每个 integration 报错信息都在 `message` 字段。
- **graceful shutdown 跑超时** —— 默认 30s drain budget，由
  `runtime.shutdown.max_drain_secs` 控制。每个 integration 的 shutdown
  又被 `resilience.shutdown_timeout_ms` (默认 5s) 单独限。
