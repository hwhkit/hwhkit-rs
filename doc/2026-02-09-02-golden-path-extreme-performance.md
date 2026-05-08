# HwhKit Golden Path：极致性能与最小可变面

日期：2026-02-09  
索引：02  
状态：建议定稿

## 1. 产品目标（明确边界）

HwhKit 的定位不是“另一个 Web 框架”，而是：

1. 固化 80%-90% 不变工程能力（配置、观测、连接管理、启动流程、健康检查、错误模型）。
2. 业务项目仅保留最小可变部分（路由注册、领域服务、DTO）。
3. 默认就是生产可用，并以性能为优先约束。

## 2. 统一技术栈（默认最佳解）

在“成熟度 + 性能 + 可维护性”维度下，默认推荐：

1. HTTP: `axum + hyper + tokio`
2. SQL: `sqlx`（postgres/mysql/sqlite）
3. Redis/Dragonfly: `redis`（multiplexed async）
4. ClickHouse: `clickhouse`
5. DuckDB: `duckdb`（本地分析）
6. Neo4j: `neo4rs`
7. MongoDB: `mongodb`
8. Meilisearch: `meilisearch-sdk`
9. Elasticsearch: `elasticsearch`（官方）
10. NATS: `async-nats`
11. Kafka: `rdkafka`（底层 librdkafka，吞吐优先）
12. MQTT: `rumqttc`
13. Qdrant: `qdrant-client`
14. Milvus / Nebula: 先 `experimental`（接口先稳定，具体 SDK 可替换）
15. Observability: `tracing + tracing-subscriber + tracing-opentelemetry + opentelemetry-otlp`

## 3. 架构基线（库内沉淀）

## 3.1 固定启动管线（不可变）

统一 `bootstrap` 流程，项目不允许自行拼接启动顺序：

1. 加载分层配置（default -> env -> env vars -> flags）
2. 严格校验（配置、feature、依赖一致性）
3. 初始化日志与 tracing
4. 初始化 integrations（按 feature + enabled）
5. 注入 `AppContext`
6. 组装 HTTP 路由与中间件
7. 启动服务 + 优雅关闭

建议暴露单一入口：

```rust
pub async fn run<A: Application>(app: A) -> Result<()>;
```

业务项目只实现 `Application` trait。

## 3.2 集成插件接口（低耦合）

每类中间件实现统一接口：

```rust
pub trait IntegrationProvider: Send + Sync {
    fn key(&self) -> &'static str;
    fn feature(&self) -> &'static str;
    fn enabled(&self, cfg: &Config) -> bool;
    async fn init(&self, ctx: &mut AppContext, cfg: &Config) -> Result<()>;
    async fn readiness(&self, ctx: &AppContext) -> Readiness;
}
```

要点：

1. 初始化顺序可配置（如 DB 先于 Repository）。
2. 每个 provider 独立失败策略：`required | optional`。
3. readiness 统一汇总到 `/readyz`。

## 3.3 `AppContext`（类型安全 + 零业务侵入）

HwhKit 提供标准上下文容器：

1. 内置强类型句柄：`PgPoolHandle`、`RedisHandle`、`KafkaProducerHandle` 等。
2. 路由层通过 `State<AppContext>` 获取，不用全局变量。
3. 未启用 feature 的句柄在编译期不可见，避免运行时分支。

## 4. 极致性能策略（默认开启）

## 4.1 编译与依赖

1. 全部外部能力 feature gate，默认仅核心 HTTP + 日志。
2. 重型依赖不进 default（如 kafka、es、milvus）。
3. 禁止在请求热路径中使用动态分发和字符串反射。

## 4.2 运行时

1. Tokio 参数可配置（worker_threads、max_blocking_threads）。
2. HTTP keepalive、request body limit、timeout、backpressure 统一模板化。
3. 连接池参数按驱动暴露（最小/最大连接、超时、空闲回收）。

## 4.3 观测开销控制

1. tracing 默认结构化字段，避免频繁格式化字符串。
2. 生产默认比例采样；错误链路强制采样。
3. access log 支持路径级白名单/黑名单（跳过健康探针）。

## 4.4 I/O 与序列化

1. 默认 `serde_json`，可选 `simd-json` feature（实验）。
2. 大 payload 场景支持压缩层与响应缓存层（按路由配置）。

## 5. 最小可变工程模板（init 生成）

`cargo hwhkit init` 生成项目只保留：

1. `src/main.rs`：调用 `hwhkit::bootstrap::run(MyApp)`
2. `src/app.rs`：实现 `Application`（注册路由、注入业务服务）
3. `src/routes/*.rs`：HTTP handler
4. `src/domain/*.rs`：领域逻辑（唯一核心可变层）
5. `config/default.toml` + `config/{dev,prod}.toml`
6. `tests/smoke.rs`：启动与健康检查

## 6. 配置与 feature 一致性约束（关键）

必须提供启动期强校验：

1. 配置启用但 feature 未打开 -> 启动失败并提示 `cargo` feature 名。
2. feature 打开但配置未启用 -> 仅告警，不初始化。
3. 关键依赖初始化失败且 `required=true` -> 启动失败。
4. `required=false` -> 降级启动，`/readyz` 显示 degraded。

## 6.1 自建配置中心（新增）

配置来源扩展为：

1. `config/default.toml`
2. `config/{env}.toml`
3. 环境变量
4. 自建配置中心 `remote patch`

原则：

1. 本地配置可独立启动（配置中心故障不阻断服务）。
2. 远程仅覆盖白名单字段，避免破坏核心连接配置。
3. 热更新仅限安全字段（日志级别、采样、限流）。

## 7. 企业级默认能力（建议内建）

1. `/healthz`（进程存活）
2. `/readyz`（依赖就绪）
3. `/metrics`（Prometheus，后续可加）
4. `request_id` 注入和透传
5. 统一错误响应模型（code/message/request_id/details）
6. 优雅关闭（SIGTERM，等待 in-flight 请求）
7. 启动报告（版本、feature、配置来源、依赖状态）

## 8. 版本策略与稳定性等级

每个集成标记稳定性：

1. `stable`: postgres/mysql/sqlite/redis/mongodb/meilisearch/elasticsearch/nats/kafka/mqtt/qdrant
2. `beta`: clickhouse/duckdb/neo4j
3. `experimental`: milvus/nebula

策略：

1. `stable` 才可进入默认模板选择器。
2. `beta/experimental` 默认不启用，需显式 `--allow-experimental`。

## 9. 落地优先级（按收益排序）

1. `Config v2 + strict validate + feature mapping`
2. `bootstrap 固定启动管线 + AppContext`
3. `observability（日志 + tracing + request_id）`
4. `stable integrations` 首批（postgres/redis/mongodb/nats/qdrant）+ `neo4j(beta)`
5. `init 命令 + 标准模板`
6. `beta/experimental integrations`

## 10. 你这条路线的核心价值

1. 新项目能在 10 分钟内启动生产级骨架。
2. 团队项目结构高度一致，运维与排障成本显著下降。
3. 默认高性能，不依赖业务开发者“每次重新做正确选择”。
