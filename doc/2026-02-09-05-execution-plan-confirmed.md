# HwhKit 确定执行方案（按评审意见）

日期：2026-02-09  
索引：05  
状态：执行中

## 1. 已确认架构

采用“改进版方案 B”：

1. workspace + facade 单入口（`hwhkit`）。
2. `hwhkit-config` 独立（零框架依赖）。
3. `hwhkit-core` 聚焦 bootstrap/error/health/web 基础设施。
4. `hwhkit-observability` 独立承载 logging/tracing。
5. 每个 integration 独立 crate。
6. 预留 `hwhkit-macros`。
7. `cargo-hwhkit` 负责 `cargo hwhkit init`。

## 2. API 契约优先（第零步）

先锁公开契约，再做内部重构：

1. `Application` trait
2. `BootstrapConfig`
3. `IntegrationProvider` trait
4. 合同测试（contract tests）

验收标准：

1. 契约在 `hwhkit-core` 对外公开。
2. 合同测试可独立运行并通过（受当前网络限制，已落测试代码，待联网验证）。

## 3. 分阶段执行

## Phase 1（已开始）

1. 建立 workspace 结构。
2. 新建核心 crate 骨架：
   - `hwhkit-config`
   - `hwhkit-core`
   - `hwhkit-observability`
   - `hwhkit-transport`
   - `hwhkit-macros`
   - `hwhkit-integration-{postgres,redis,mongodb,nats,qdrant,neo4j}`
   - `cargo-hwhkit`
3. `hwhkit` facade 保持兼容并预留 v2 re-export。

## Phase 2（下一步）

1. `hwhkit-config` 实现分层加载（default + env + env vars + remote patch）。
2. `hwhkit-core` 实现固定 bootstrap pipeline。
3. 引入 strict validate（配置与 feature 一致性校验）。

## Phase 3

1. 首批 integrations 真正接入并初始化：
   - postgres
   - redis/dragonfly
   - mongodb
   - nats
   - qdrant
   - neo4j

## Phase 4

1. `hwhkit-transport` 实现：
   - service-to-service: gRPC + NATS RPC + NATS event
   - edge: REST + gRPC + WebSocket
2. `p2p` 保持 experimental 占位。

## Phase 5

1. `cargo hwhkit init` 模板完善：
   - `.env.example`
   - `Dockerfile`
   - `.github/workflows/ci.yml`
   - `README.md`
   - `justfile`
2. 模板参数化（minimal-api/api-grpc/realtime-event）。

## 4. 当前已落地内容

1. workspace 根配置已创建。
2. 上述 crate 目录和最小可编译骨架已创建。
3. `Application`、`BootstrapConfig`、`IntegrationProvider` 已定义。
4. `hwhkit-core/tests/api_contract.rs` 合同测试已新增。
5. `cargo-hwhkit init` 最小可用模板生成已实现第一版。

## 5. 当前状态

1. `crates.io` 依赖下载与关键测试已打通，基础验证通过。
2. 当前进入下一阶段：补 transport 可运行骨架与模板真实接线。
