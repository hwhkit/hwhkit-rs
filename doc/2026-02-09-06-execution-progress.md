# HwhKit 执行进度记录

日期：2026-02-09  
索引：06  
状态：持续更新

## 记录规则

每完成一步记录：

1. 目标
2. 结果
3. 变更文件
4. 下一步

## Step 1（已完成）：`hwhkit-config` 分层加载 + Remote 配置中心接入抽象

### 目标

实现 `default + env + env vars + remote patch` 的配置装载能力，并支持“自建配置中心可接入、不可用可降级”。

### 结果

1. 新增 `AppConfig` 配置模型（server/observability/integrations/transport）。
2. 新增 `ConfigLoader`，默认 source 顺序为：
   - `file:default`
   - `file:environment`
   - `env`
3. 新增 `RemoteConfigProvider` + `RemoteConfigSource` + `RemotePatchPolicy`。
4. remote patch 采用白名单字段过滤（默认仅允许 observability 安全字段）。
5. 新增 `validate_strict`，校验关键字段和启用项必填值。
6. 补充单元测试覆盖分层合并与 remote 白名单过滤。

### 变更文件

1. `crates/hwhkit-config/Cargo.toml`
2. `crates/hwhkit-config/src/lib.rs`

### 下一步

把 loader + strict validate 串入 `hwhkit-core` 固定 bootstrap 管线，并加 feature/config 一致性校验。

## Step 2（已完成）：`hwhkit-core` 固定 bootstrap 管线 + feature/config 校验

### 目标

落地核心启动流程：加载配置 -> 校验 feature 映射 -> 初始化 integration -> 构建 router。

### 结果

1. 重构 `Application` 契约：`build_router(&AppConfig)`。
2. 增加 `RuntimeFeatures`，用于运行时传入已启用 feature 状态。
3. 新增 `bootstrap_with(...)`：
   - 使用 `ConfigLoader` 读取配置
   - 执行 `validate_feature_mapping`
   - 初始化 `IntegrationProvider`
   - 记录 `initialized_integrations` 与 `degraded_integrations`
4. 新增 feature mismatch 明确错误提示（例如 `transport.rpc.default=grpc` 但未开 `transport-grpc`）。
5. 合同测试更新为真实配置文件驱动。

### 变更文件

1. `crates/hwhkit-core/src/lib.rs`
2. `crates/hwhkit-core/tests/api_contract.rs`

### 下一步

升级 `cargo hwhkit init` 为多模板生成，覆盖 `minimal-api / api-grpc / realtime-event`。

## Step 3（已完成）：`cargo hwhkit init` 多模板生成

### 目标

按既定模板生成最小可变项目骨架，并覆盖运维基础文件。

### 结果

1. `--template` 改为强类型枚举（`minimal-api`、`api-grpc`、`realtime-event`）。
2. 三模板分别生成不同的 features 和骨架文件：
   - `minimal-api`：标准 API 起步
   - `api-grpc`：含 `proto/` 与 `scripts/gen-proto.sh`
   - `realtime-event`：含 websocket/nats 事件占位
3. 共通文件统一生成：
   - `.env.example`
   - `Dockerfile`
   - `.github/workflows/ci.yml`
   - `README.md`
   - `justfile`
   - `tests/smoke.rs`

### 变更文件

1. `crates/cargo-hwhkit/src/main.rs`

### 下一步

开始 Phase 3：实现首批 integration crate 的真实初始化逻辑（postgres/redis/mongodb/nats/qdrant/neo4j）并接入 `IntegrationProvider`。

## Step 4（已完成）：首批 integration crate 接入 `IntegrationProvider`

### 目标

把首批六个 integration 从“配置结构体占位”升级为可进入 bootstrap 管线的 provider。

### 结果

1. 六个 integration crate 统一依赖 `hwhkit-core + hwhkit-config + async-trait`。
2. 每个 crate 都实现了：
   - `*Provider`（实现 `IntegrationProvider`）
   - `*Handle`（注入到 `AppContext`）
   - 初始化参数基本校验（URL 前缀、关键字段）
3. 失败策略遵循配置中的 `required` 字段。
4. facade feature 与独立 crate 完成映射，保持单入口体验。

### 变更文件

1. `crates/hwhkit-integration-postgres/Cargo.toml`
2. `crates/hwhkit-integration-postgres/src/lib.rs`
3. `crates/hwhkit-integration-redis/Cargo.toml`
4. `crates/hwhkit-integration-redis/src/lib.rs`
5. `crates/hwhkit-integration-mongodb/Cargo.toml`
6. `crates/hwhkit-integration-mongodb/src/lib.rs`
7. `crates/hwhkit-integration-nats/Cargo.toml`
8. `crates/hwhkit-integration-nats/src/lib.rs`
9. `crates/hwhkit-integration-qdrant/Cargo.toml`
10. `crates/hwhkit-integration-qdrant/src/lib.rs`
11. `crates/hwhkit-integration-neo4j/Cargo.toml`
12. `crates/hwhkit-integration-neo4j/src/lib.rs`
13. `crates/hwhkit/Cargo.toml`
14. `src/lib.rs`

### 下一步

将上述 providers 接入 facade 的标准 provider 注册器，并补一条 bootstrap 集成测试。

## Step 5（已完成）：facade 标准启动入口 `run_v2` 与 provider 自动注册

### 目标

让业务项目无需重复装配 `ConfigLoader + RuntimeFeatures + Providers`，直接调用统一启动入口。

### 结果

1. 新增 `src/bootstrap_v2.rs`：
   - `runtime_features()`：基于 compile-time feature 自动构建 `RuntimeFeatures`
   - `default_providers()`：按 feature 自动注册六个 integration provider
   - `run(...)`：统一调用 `hwhkit_core::bootstrap_with(...)`
2. `hwhkit` facade 增加导出：
   - `pub mod bootstrap_v2`（`config-v2` feature 下）
   - `pub use bootstrap_v2::run as run_v2`

### 变更文件

1. `src/bootstrap_v2.rs`
2. `src/lib.rs`

### 下一步

补充一条 `run_v2` 路径的集成测试与示例模板主程序接入。

## Step 6（已完成）：关键测试回归与文档示例阻塞修复

### 目标

验证新架构落地后的关键 crate 可编译可测试，并处理可能阻塞 CI 的 doctest 问题。

### 结果

1. 通过测试：
   - `cargo test -p hwhkit-config -p hwhkit-core`
   - `cargo test -p cargo-hwhkit`
   - `cargo test -p hwhkit`
   - `cargo test -p hwhkit-integration-{postgres,redis,mongodb,nats,qdrant,neo4j}`
2. 修复 doctest 卡住问题：
   - 将会真实启动服务的示例改为 `no_run`。
3. 当前阶段已具备“可持续迭代”的基础验证闭环。

### 变更文件

1. `src/lib.rs`
2. `src/builder.rs`

### 下一步

进入 Phase 4：实现 `hwhkit-transport` 中 gRPC/NATS RPC/WebSocket 的首批可运行骨架，并把 `api-grpc`、`realtime-event` 模板主程序改为真实接入 `run_v2`。

## Step 7（已完成）：`hwhkit-transport` 抽象层升级（RPC/EventBus/WS）

### 目标

把 `hwhkit-transport` 从占位配置提升为可复用协议抽象层，支撑后续 gRPC/NATS/WS 实现接入。

### 结果

1. 新增统一 `TransportConfig`（grpc/rpc/nats/websocket/p2p）。
2. 新增传输抽象：
   - `RpcClient`
   - `EventBus`
   - `EventSubscriber`
3. 新增基础实现：
   - `MemoryEventBus`（可测试）
   - `GrpcRpcClient`（占位）
   - `NatsRpcClient`（占位）
4. 增加 transport 单测：`memory_event_bus_roundtrip`。

### 变更文件

1. `crates/hwhkit-transport/Cargo.toml`
2. `crates/hwhkit-transport/src/lib.rs`

### 下一步

把模板主程序接入 `run_v2`，并在 `cargo-hwhkit` 中完成三模板统一生成。

## Step 8（已完成）：模板主程序接线 `run_v2` + CLI 路径名修复

### 目标

让新项目模板从“占位打印”变为“真实 bootstrap 接线”，并修复路径型项目名生成问题。

### 结果

1. 三模板统一改为生成：
   - `src/main.rs` 调用 `run_v2`
   - `src/app.rs` 实现 `Application`（健康检查路由）
2. `Cargo.toml` 模板增加 `async-trait`。
3. 修复 `cargo hwhkit init /abs/path` 场景下 package name 和 Dockerfile 二进制名错误。
4. CLI 冒烟验证通过：
   - `minimal-api`
   - `api-grpc`
   - `realtime-event`
   - 绝对路径 project name 场景

### 变更文件

1. `crates/cargo-hwhkit/src/main.rs`

### 下一步

进入下一轮：将 transport 的 gRPC/NATS/WebSocket 占位实现替换为真实驱动接入（tonic/async-nats/axum ws）。

## Step 9（已完成）：回归测试收敛（workspace 关键包）

### 目标

确认本轮重构后的核心链路可编译、可测试、可持续迭代。

### 结果

1. 通过统一回归命令：

```bash
cargo test -p hwhkit-config -p hwhkit-core -p hwhkit-transport -p cargo-hwhkit -p hwhkit
```

2. 所有目标包测试通过，doctest 通过。
3. 当前残余仅为示例文件中的 unused warning（不影响功能正确性）。

### 变更文件

本步骤无新增代码文件，仅执行验证。

### 下一步

进入下一阶段：把 transport 抽象替换为真实 `tonic + async-nats + axum ws` 实现，并对 `api-grpc/realtime-event` 模板补端到端样例。
