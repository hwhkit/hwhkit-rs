# HwhKit v2 架构升级方案（草案）

日期：2026-02-09  
索引：01  
状态：已确认（2026-02-09）

## 1. 目标

围绕以下目标升级 HwhKit：

1. 配置能力覆盖主流数据存储、检索、消息系统与向量数据库。
2. 通过 Cargo feature 按需启用依赖，避免默认引入重量级组件。
3. 集成生产级日志与链路追踪（OpenTelemetry 生态）。
4. 提供项目初始化能力（主入口 `cargo hwhkit init`）。
5. 保持清晰、低耦合、可测试、可演进。

## 2. 总体设计原则

1. 分层解耦：`config`（声明）与 `integration`（连接实现）分离。
2. 编译期可选：所有外部中间件通过 feature gate 控制。
3. 运行期可控：配置中 `enabled` 开关 + 初始化失败策略可配置。
4. 可观察优先：所有集成初始化、重连、失败、降级都输出结构化日志和 span。
5. 失败可诊断：统一错误类型、统一启动报告。

## 3. 配置模型（Config v2）

建议新增顶层结构：

```toml
[server]
host = "0.0.0.0"
port = 3000
architecture = "api"

[observability]
service_name = "my-service"
environment = "dev"

[observability.logging]
level = "info"
format = "json"        # compact | pretty | json
request = true

[observability.tracing]
enabled = true
sampler = "parentbased_traceidratio"
sample_ratio = 1.0
otlp_endpoint = "http://127.0.0.1:4317"
otlp_protocol = "grpc" # grpc | http

[integrations.sql.postgres]
enabled = false
url = "postgres://..."
max_connections = 20

[integrations.sql.mysql]
enabled = false
url = "mysql://..."
max_connections = 20

[integrations.sql.sqlite]
enabled = false
url = "sqlite://app.db"

[integrations.redis]
enabled = false
url = "redis://127.0.0.1:6379"
# Dragonfly 使用 Redis 协议，沿用 redis 配置

[integrations.clickhouse]
enabled = false
url = "http://127.0.0.1:8123"

[integrations.duckdb]
enabled = false
path = "data/app.duckdb"

[integrations.neo4j]
enabled = false
url = "bolt://127.0.0.1:7687"
username = "neo4j"
password = "..."

[integrations.nebula]
enabled = false
address = "127.0.0.1:9669"
username = "root"
password = "nebula"

[integrations.mongodb]
enabled = false
url = "mongodb://127.0.0.1:27017"
database = "app"

[integrations.meilisearch]
enabled = false
url = "http://127.0.0.1:7700"
api_key = ""

[integrations.elasticsearch]
enabled = false
url = "http://127.0.0.1:9200"
api_key = ""

[integrations.messaging.nats]
enabled = false
url = "nats://127.0.0.1:4222"

[integrations.messaging.kafka]
enabled = false
brokers = ["127.0.0.1:9092"]

[integrations.messaging.mqtt]
enabled = false
host = "127.0.0.1"
port = 1883

[integrations.vector.milvus]
enabled = false
url = "http://127.0.0.1:19530"

[integrations.vector.qdrant]
enabled = false
url = "http://127.0.0.1:6334"
api_key = ""
```

### 3.1 配置加载策略

建议按优先级加载：

1. 代码默认值
2. `config/default.toml`
3. `config/{env}.toml`（如 `dev/prod`）
4. 环境变量（前缀 `HWHKIT__`）
5. CLI 参数（后续 CLI 引入时）

并预留“自建配置中心”接入层（见 3.3）：

1. 本地文件仍为兜底和启动必需。
2. 远端配置中心用于动态覆盖可热更新字段（日志级别、采样率、限流参数等）。
3. 配置中心不可用时自动降级为本地配置，避免启动硬失败。

### 3.3 自建配置中心接入（新增）

新增抽象：

```rust
pub trait ConfigSource: Send + Sync {
    fn name(&self) -> &'static str;
    async fn load(&self) -> Result<ConfigPatch>;
    async fn watch(&self) -> Result<ConfigStream>;
}
```

建议内置 source：

1. `FileConfigSource`（default/dev/prod）
2. `EnvConfigSource`（`HWHKIT__`）
3. `RemoteConfigSource`（HTTP/GRPC 拉取 + watch）

合并策略：

1. `base config` + `remote patch`（仅允许白名单字段覆盖）
2. 热更新字段生效，不可热更新字段仅下次重启生效

### 3.2 配置校验

新增 `Config::validate_strict()`：

1. 校验 URL/地址格式。
2. 校验 feature 与配置一致性（例如配置启用 postgres 但未启用 `db-postgres` feature）。
3. 校验互斥关系（如同一角色下不允许多主连接配置冲突）。
4. 校验敏感项非空（生产模式）。

## 4. Cargo Feature 设计

建议从“功能域”拆分：

```toml
[features]
default = ["observability-logs"]

# observability
observability-logs = []
observability-tracing = ["dep:opentelemetry", "dep:opentelemetry_sdk", "dep:opentelemetry-otlp", "dep:tracing-opentelemetry"]

# SQL
db-postgres = ["dep:sqlx", "sqlx/postgres"]
db-mysql = ["dep:sqlx", "sqlx/mysql"]
db-sqlite = ["dep:sqlx", "sqlx/sqlite"]

# cache
cache-redis = ["dep:redis"]

# OLAP
olap-clickhouse = ["dep:clickhouse"]
olap-duckdb = ["dep:duckdb"]

# graph
graph-neo4j = ["dep:neo4rs"]
# Nebula Rust SDK 生态仍在演进，先作为实验级占位 feature
graph-nebula = []

# document / search
doc-mongodb = ["dep:mongodb"]
search-meilisearch = ["dep:meilisearch-sdk"]
search-elasticsearch = ["dep:elasticsearch"]

# messaging
mq-nats = ["dep:async-nats"]
mq-kafka = ["dep:rdkafka"]
mq-mqtt = ["dep:rumqttc"]

# vector
# Milvus Rust SDK 可先用 git 依赖或 gRPC 自建封装，先占位
vector-milvus = []
vector-qdrant = ["dep:qdrant-client"]

# bundles
storage-basic = ["db-postgres", "cache-redis"]
search-basic = ["search-meilisearch"]
messaging-basic = ["mq-nats"]
vector-basic = ["vector-qdrant"]
full = ["observability-tracing", "storage-basic", "search-basic", "messaging-basic", "vector-basic"]
```

## 5. 运行时集成模型

新增 `integration` 模块，定义统一接口：

```rust
pub trait IntegrationProvider {
    fn name(&self) -> &'static str;
    fn enabled(&self, cfg: &Config) -> bool;
    async fn init(&self, ctx: &mut AppContext, cfg: &Config) -> Result<()>;
}
```

`AppContext` 存放已初始化客户端（连接池、SDK client、producer/consumer 句柄），并通过 `Extension/State` 暴露给 Axum 路由。

## 6. 观测方案（日志 + 链路）

### 6.1 日志

1. 基于 `tracing + tracing-subscriber`。
2. 支持 `pretty/compact/json` 输出。
3. 默认 JSON（生产）+ pretty（本地开发）可切换。

### 6.2 分布式追踪

1. 使用 `tracing-opentelemetry` 将 span 导出到 OTLP。
2. 使用 `opentelemetry-otlp` 对接 Collector（Jaeger/Tempo/Datadog/New Relic 等）。
3. 请求入口生成或继承 trace context，响应日志打印 trace_id/span_id。
4. 关键集成调用（DB、MQ、向量检索）加 `#[instrument]`。

### 6.3 最佳实践默认值

1. 开发环境：全量采样 + pretty 日志。
2. 生产环境：比例采样（如 1%-10%）+ JSON 日志 + 错误全采样。
3. 所有日志字段携带 `service.name`、`env`、`trace_id`、`request_id`。

## 7. CLI 初始化能力

注意：仅 `cargo add hwhkit` 不会自动安装可执行程序。要支持命令，需要二选一：

1. `cargo hwhkit init`：发布 `cargo-hwhkit` 子命令（主入口，已确认）。
2. `hwhkit init`：可作为后续兼容别名（非当前主路径）。

### 7.1 init 生成内容建议

1. 标准目录：
   - `src/main.rs`
   - `src/routes/`
   - `src/app_state.rs`
   - `config/default.toml`
   - `config/dev.toml`
   - `config/prod.toml`
   - `.env.example`
   - `tests/`
2. 可选模板参数：
   - 架构类型（api/full）
   - 数据库类型（postgres/mysql/sqlite/none）
   - 消息系统（nats/kafka/mqtt/none）
   - 观测默认（otel on/off）
3. 生成后自动提示：
   - 启用哪些 cargo features
   - 下一步命令（`cargo run`, `cargo test`）

## 8. 还需要补充的关键能力

1. 健康检查和就绪检查：`/healthz`、`/readyz`，并聚合各中间件状态。
2. 启动诊断报告：打印 feature、配置来源、启用的集成清单。
3. 连接生命周期管理：重连策略、超时、熔断、优雅关闭。
4. 配置热更新策略（可选）：先支持日志级别热更新。
5. 基准与兼容矩阵：记录每个集成的稳定级别（stable/experimental）。
6. 安全基线：敏感配置脱敏输出、默认禁止弱密码示例。

## 9. 分阶段落地（建议）

1. Phase 1：Config v2 + feature gate 重构 + 文档样例。
2. Phase 2：首批稳定链路 `postgres + redis + mongodb + nats + qdrant + neo4j`。
3. Phase 3：ClickHouse/DuckDB/Milvus/Nebula（其中 Nebula、Milvus 标记 experimental）。
4. Phase 4：`cargo-hwhkit` + `hwhkit-cli` init 模板生成。
5. Phase 5：完善示例、CI、端到端测试和 benchmark。

## 10. 待确认决策

1. `init` 主入口：`cargo hwhkit init`（已确认）。
2. Nebula 与 Milvus：`experimental`（已确认）。
3. 配置文件：分层 `config/default + env`（已确认）。
4. 配置中心：支持自建 remote source（已新增）。
5. 是否引入兼容层（保留旧版 `middleware.*` 配置并自动映射到 v2）待定。

## 11. 参考来源（选型依据）

1. Cargo 自定义子命令机制：<https://doc.rust-lang.org/stable/cargo/reference/external-tools.html>
2. SQLx（Postgres/MySQL/SQLite）：<https://docs.rs/crate/sqlx/latest>
3. Redis 客户端：<https://docs.rs/redis/latest/redis/>
4. Dragonfly 与 Redis 协议兼容：<https://www.dragonflydb.io/docs>
5. ClickHouse Rust 客户端：<https://docs.rs/clickhouse/latest/clickhouse/>
6. DuckDB Rust 客户端：<https://duckdb.org/docs/stable/clients/rust>
7. Neo4j Rust 驱动 neo4rs：<https://docs.rs/neo4rs>
8. MongoDB 官方 Rust Driver：<https://docs.rs/mongodb/latest/mongodb/>
9. Meilisearch Rust SDK：<https://github.com/meilisearch/meilisearch-rust>
10. Elasticsearch 官方 Rust 客户端：<https://docs.rs/elasticsearch/>
11. NATS async 客户端：<https://docs.rs/async-nats/>
12. Kafka rust-rdkafka：<https://docs.rs/rdkafka/>
13. MQTT rumqttc：<https://docs.rs/rumqttc/latest/rumqttc/>
14. Qdrant Rust 客户端与官方接口文档：<https://docs.rs/qdrant_client>、<https://qdrant.tech/documentation/interfaces/>
15. Nebula Graph 文档：<https://docs.nebula-graph.io/>
16. Milvus Rust SDK：<https://github.com/milvus-io/milvus-sdk-rust>
17. tracing：<https://docs.rs/tracing/>
18. tracing-opentelemetry：<https://docs.rs/tracing-opentelemetry/latest/tracing_opentelemetry/>
19. OpenTelemetry Rust：<https://opentelemetry.io/docs/languages/rust/>
