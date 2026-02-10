# HwhKit

HwhKit 是一个面向 Rust Web 服务的工程化脚手架库，目标是把不变的基础设施沉淀到框架层，让业务项目只保留最小可变部分。

当前仓库已升级为 workspace 架构（`0.2.0-alpha.1`），提供：

1. 兼容层：原有 `WebServerBuilder`（v1 风格）继续可用。
2. 新架构：`config-v2 + core-v2 + run_v2` 启动管线。
3. CLI：`cargo hwhkit init` 生成标准项目模板。

## 当前能力（按代码现状）

1. workspace 分层拆包：`hwhkit-config`、`hwhkit-core`、`hwhkit-observability`、`hwhkit-transport`、`hwhkit-macros`、`hwhkit-integration-*`、`cargo-hwhkit`。
2. 配置分层加载：`config/default.toml -> config/{env}.toml -> ENV(HWHKIT__) -> remote patch`。
3. 严格校验：配置合法性 + feature/config 一致性校验。
4. 首批集成 provider：`postgres/redis/mongodb/nats/qdrant/neo4j`（当前为初始化骨架与参数校验、Handle 注入）。
5. 传输层抽象：`RPC/EventBus/WebSocket/P2P` 配置模型与接口；`MemoryEventBus` 可用。
6. 模板初始化：`minimal-api`、`api-grpc`、`realtime-event`。

## 仓库结构

```text
hwhkit-rs/
  crates/
    hwhkit/                         # facade：对外唯一依赖入口
    hwhkit-config/                  # 配置加载/合并/校验
    hwhkit-core/                    # bootstrap/Application/IntegrationProvider
    hwhkit-observability/           # logging/tracing 初始化
    hwhkit-transport/               # gRPC/RPC/WS/P2P 抽象层
    hwhkit-macros/                  # proc-macro 预留
    hwhkit-integration-postgres/
    hwhkit-integration-redis/
    hwhkit-integration-mongodb/
    hwhkit-integration-nats/
    hwhkit-integration-qdrant/
    hwhkit-integration-neo4j/
    cargo-hwhkit/                   # cargo hwhkit init
  src/                              # facade 兼容入口（v1 + v2 re-export）
  templates/                        # 模板目录
  examples/
  doc/
```

## Feature 概览（`crates/hwhkit/Cargo.toml`）

基础：

- `templates`
- `jwt`
- `config-v2`
- `macros`

传输：

- `transport-grpc`
- `transport-ws`
- `transport-p2p`

集成：

- `postgres`
- `redis`
- `mongodb`
- `nats`
- `qdrant`
- `neo4j`

聚合：

- `full`

## 安装与使用

### 1) 作为库使用（推荐）

```toml
[dependencies]
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
hwhkit = { version = "0.2.0-alpha.2", features = [
  "config-v2",
  "transport-grpc",
  "transport-ws",
  "postgres",
  "redis",
  "mongodb",
  "nats",
  "qdrant",
  "neo4j"
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

## 快速开始（v2）

```rust
use async_trait::async_trait;
use hwhkit::{
    config_v2::{AppConfig, BootstrapConfig},
    core_v2::{AppContext, Application, Result},
    get, run_v2, Router,
};

struct App;

#[async_trait]
impl Application for App {
    async fn build_router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new().route("/healthz", get(health)))
    }
}

async fn health() -> &'static str {
    "ok"
}

#[tokio::main]
async fn main() {
    let bootstrap = BootstrapConfig::default();
    let built = run_v2(App, bootstrap).await.expect("bootstrap failed");

    println!("applied_sources = {:?}", built.applied_sources);
    println!("initialized_integrations = {:?}", built.initialized_integrations);
    println!("degraded_integrations = {:?}", built.degraded_integrations);

    // built.router 可继续接入你自己的 server runtime
}
```

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

[transport.grpc]
enabled = false
listen = "0.0.0.0:50051"

[transport.rpc]
enabled = false
default = "grpc"
timeout_ms = 3000

[transport.websocket]
enabled = false
path = "/ws"
max_connections = 10000
heartbeat_seconds = 20
```

## 兼容模式（v1）

原有 `WebServerBuilder` 仍可使用：

```rust
use hwhkit::WebServerBuilder;

#[tokio::main]
async fn main() {
    let server = WebServerBuilder::new()
        .config_from_file("config.toml")
        .build()
        .await
        .expect("failed to build");

    server.serve().await.expect("failed to serve");
}
```

## 质量状态

已通过的关键测试：

```bash
cargo test -p hwhkit-config -p hwhkit-core -p hwhkit-transport -p cargo-hwhkit -p hwhkit
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

1. `hwhkit-integration-*` 当前是 provider 骨架与参数校验层，真实驱动深度接入在后续阶段推进。
2. `hwhkit-transport` 当前提供稳定抽象与可测试基础实现，`tonic/async-nats/axum ws` 的完整运行时实现仍在计划中。
3. `transport-p2p` 保持 experimental。

## License

MIT OR Apache-2.0
