# HwhKit V3 Runtime Architecture

本文定义 HwhKit 下一阶段的主线目标：从“Web 服务脚手架”升级为“Rust 服务开发框架与运行时”。

## 核心目标

1. 统一应用启动模型：配置、生命周期、依赖注入、观测、健康检查、优雅退出。
2. 统一资源模型：SQL、KV、消息、搜索、向量库通过一致的注册与获取方式暴露给业务层。
3. 统一通信模型：HTTP、WebSocket、RPC、NATS、Socket 通过统一的服务调用语义接入。
4. 统一服务编排模型：本地模块与远程服务都可通过同一套 `ServiceTarget` 与 `ServiceClient` 调用。

## 分层

### `hwhkit-config`

- 负责 layered config、严格校验、远端 patch。
- 新增 `resources` 与 `mesh` 模型。
- `resources` 描述资源绑定，不直接等同于 integration crate。
- `mesh.services` 描述服务访问方式，而不是业务模块实现方式。

### `hwhkit-core`

- `AppContext`: 业务上下文。
- `ResourceRegistry`: 统一资源注册中心。
- `ServiceRegistry`: 本地服务与远程客户端的统一访问入口。
- `ServiceTarget`: 支持 `Local / Remote / Auto`。

### `hwhkit-transport`

- `ProtocolKind`: `Http / WebSocket / Rpc / Nats / Socket`
- `CommunicationPattern`: `RequestResponse / Stream / PublishSubscribe / FireAndForget`
- `ProtocolAdapter`: 协议适配接口。
- `AdapterRegistry`: 协议适配器注册中心。

### Integration crates

- 负责 provider 初始化与句柄注入。
- 初始化后必须同时：
  - 继续将原生句柄注入 `AppContext::insert`
  - 将资源按统一命名注册到 `ResourceRegistry`

## 统一调用语义

业务层应优先面向调用语义，而不是具体协议：

- 同步请求：`ServiceRequest -> ServiceResponse`
- 本地优先：`ServiceTarget::auto("user")`
- 强制远程：`ServiceTarget::remote("user")`
- 强制本地：`ServiceTarget::local("user")`

后续协议适配规则：

- HTTP: Request/Response
- RPC: Request/Response, Stream
- NATS: Request/Response, Publish/Subscribe
- WebSocket: Stream, Fire-and-forget
- Socket: Stream

## 资源命名约定

第一阶段的默认命名约定：

- SQL: `main`
- KV: `cache`
- Vector: `default`
- MessageBus: `default`
- Document/Graph 等扩展资源使用 `ResourceKind::custom(...)`

后续将允许从配置中覆盖默认资源别名。

## 第一阶段已落地骨架

本次改造完成了以下底座：

1. `hwhkit-config` 已支持 `resources` 与 `mesh` 配置模型。
2. `hwhkit-core` 已支持 `ResourceRegistry`、`ServiceRegistry`、`ServiceTarget`。
3. `hwhkit-core` 已定义第一版统一资源 trait：`SqlDatabase`、`KvStore`、`VectorStore`、`SearchEngine`、`MessageBusResource`。
4. `hwhkit-transport` 已支持 `ProtocolAdapter`、`AdapterRegistry` 与配置驱动的 `MeshClient`。
5. 现有 integration provider 初始化后已注册统一资源句柄。

## 下一阶段

1. 增加真正的 `sql-postgres / sql-mysql / kv-redis / mq-nats / vector-qdrant` runtime provider。
2. 增加 `MeshClient`，把 `mesh.services.*` 解析为真实远程调用。
3. 在 facade crate 上暴露模块化 API：`Module / Resource / Service`。
4. 把 `v1 builder` 明确降级为兼容层，不再承载新增能力。
