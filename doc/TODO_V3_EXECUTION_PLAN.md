# HwhKit V3 TODO Execution Plan

本文件是 HwhKit 从“Web 脚手架”升级为“Rust 服务开发框架与运行时”的执行清单。

执行原则：

1. 只扩展 `v2/v3` 主线，`v1 builder` 进入兼容层。
2. 先做统一抽象，再接具体 provider。
3. 每个阶段都要有可验证产物，避免只写接口不形成主链路。

状态标记：

- `[x]` 已完成
- `[-]` 进行中
- `[ ]` 未开始

## Phase 0: Groundwork

- [x] 建立 `resources + mesh + protocol adapters` 核心骨架
  - 验收标准：存在 `ResourceRegistry`、`ServiceRegistry`、`ProtocolAdapter` 与文档
- [x] 为现有 provider 注册统一资源句柄
  - 验收标准：postgres/redis/nats/qdrant 等在初始化后可通过统一资源名获取
- [x] 补充 v3 架构文档
  - 验收标准：存在明确分层、目标和下一阶段计划

## Phase 1: Naming And Feature Alignment

- [x] 收敛 facade feature 命名，增加 v3 风格别名
  - 目标：引入 `sql-postgres`、`kv-redis`、`vector-qdrant`、`mq-nats`、`transport-rpc`、`transport-http`、`transport-socket`
  - 验收标准：新旧 feature 可并存，README 和文档更新为新命名优先
- [x] 收敛 transport feature 命名
  - 目标：从 `grpc/ws/p2p` 过渡到更稳定的能力命名
  - 验收标准：transport crate 支持 `http/ws/rpc/socket` 风格特性
- [x] 在 README 中明确“当前可用”和“规划中”的边界
  - 验收标准：README 不再把占位实现描述成完整能力

## Phase 2: Mesh Mainline

- [x] 实现配置驱动的 `MeshClient`
  - 目标：读取 `mesh.services.*`，决定 `Local/Remote/Auto` 调用路线
  - 验收标准：存在从 `AppConfig.mesh` 构建 client 的代码
- [x] 实现协议路由器
  - 目标：基于 `protocol` 选择 `ProtocolAdapter`
  - 验收标准：`rpc/http/nats/ws/socket` 可以通过统一入口分发
- [x] 接通 loopback/local-first 逻辑
  - 目标：同进程注册服务优先本地调用，否则降级远程
  - 验收标准：单测覆盖 `auto -> local` 与 `auto -> remote`

## Phase 3: Resource Runtime Abstractions

- [x] 定义统一资源 trait
  - 目标：`SqlDatabase`、`KvStore`、`VectorStore`、`SearchEngine`、`MessageBus`
  - 验收标准：核心 trait 位于稳定模块，并带最小能力集
- [x] 提供 provider-native escape hatch
  - 目标：统一抽象之外仍能拿到底层原生句柄
  - 验收标准：文档与示例说明 `native handle` 获取方式
- [ ] 增加资源工厂接口
  - 目标：从配置创建资源实例，而不是仅做参数校验
  - 验收标准：存在 `ResourceFactory` 或等价抽象

## Phase 4: First Real Providers

- [x] 落地 `sql-postgres`
  - 验收标准：真实连接池、健康检查、基础查询接口
- [x] 落地 `kv-redis`
  - 验收标准：真实客户端、基础 get/set/del、健康检查
- [x] 落地 `mq-nats`
  - 验收标准：publish/subscribe/request-reply 主链路可跑通
- [x] 落地 `vector-qdrant`
  - 验收标准：collection/create/upsert/search 最小功能可跑通

## Phase 5: Service Runtime Experience

- [x] 引入 `Module` 抽象
  - 目标：模块可注册 route/service/resource/health checks
  - 验收标准：至少一个 demo app 使用 `Module` 组织代码
- [x] 提供统一入口 runtime builder
  - 目标：替代零散 `run_v2` 启动方式
  - 验收标准：新 builder 能连接 config/resources/mesh/transports
- [x] 增加示例工程
  - 目标：演示 HTTP ingress + NATS service-to-service + WebSocket stream
  - 验收标准：examples 至少包含一个端到端演示

## Phase 6: Expanded Ecosystem

- [ ] `sql-mysql`
- [ ] `vector-milvus`
- [ ] `mq-kafka`
- [ ] `mq-mqtt`
- [ ] `search-elasticsearch`
- [ ] `search-meilisearch`
- [ ] `kv-dragonfly`

验收标准：

- 每个 provider 至少包含真实初始化、健康检查、统一接口适配、原生句柄暴露

## Phase 7: Hardening

- [ ] 观测：trace/span/metrics 主链路
- [ ] retry/timeout/circuit-breaker 策略
- [ ] 配置热更新策略
- [ ] 生命周期与优雅退出
- [ ] 更强的集成测试矩阵

## Immediate Next Actions

- [x] 1. 完成 feature 命名收敛与文档同步
- [x] 2. 实现 `MeshClient` 骨架
- [x] 3. 实现 `MeshClient` 与 `AdapterRegistry` 的连接
- [x] 4. 设计统一资源 trait
- [x] 5. 落地首个真实 provider：`mq-nats`
- [x] 6. 让 `RuntimeBuilder` 自动接通 `mesh.services` 与 `MeshClient`
- [x] 7. 补齐 `Module` + runtime 的本地 mesh demo
- [x] 8. 补齐 HTTP ingress + NATS + WebSocket 的完整示例工程
  - 当前状态：example + real NATS smoke/e2e test 已落地
