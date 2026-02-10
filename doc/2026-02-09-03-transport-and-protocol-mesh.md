# HwhKit 传输与协议网格方案（Service-to-Service + Edge）

日期：2026-02-09  
索引：03  
状态：已确认范围

## 1. 目标

同时满足两类场景：

1. Service-to-Service：快速构建 `gRPC/RPC`、`NATS`、`WebSocket` 事件传递。
2. 对外服务（Edge）：支持 `RESTful`、`RPC`、`WebSocket`、`P2P`。

## 2. 协议分层（默认架构）

统一分为三层：

1. `Edge API Layer`：对外入口（REST/gRPC/WebSocket/P2P 网关）
2. `Service Mesh Layer`：服务间调用（gRPC + NATS request/reply）
3. `Event Fabric Layer`：异步事件（NATS pub/sub + 可选 WebSocket fanout）

## 3. 默认技术选型（性能优先）

1. REST：`axum`（HTTP/1.1 + HTTP/2）
2. gRPC：`tonic`（service-to-service 主 RPC）
3. Rust RPC：`tonic` 统一接口；可选 `NATS request/reply` 做轻量 RPC
4. Event Bus：`async-nats`
5. WebSocket：`axum::extract::ws`
6. P2P：`libp2p`（`experimental`）

## 4. HwhKit 统一抽象

## 4.1 传输能力注册

```rust
pub trait TransportProvider: Send + Sync {
    fn key(&self) -> &'static str;
    fn feature(&self) -> &'static str;
    fn enabled(&self, cfg: &Config) -> bool;
    async fn init(&self, ctx: &mut AppContext, cfg: &Config) -> Result<()>;
}
```

## 4.2 RPC 抽象

```rust
pub trait RpcClient {
    async fn call<Req, Resp>(&self, method: &str, req: Req) -> Result<Resp>;
}
```

实现：

1. `GrpcRpcClient`（tonic）
2. `NatsRpcClient`（request/reply）

## 4.3 事件抽象

```rust
pub trait EventBus {
    async fn publish<E: Event>(&self, topic: &str, event: E) -> Result<()>;
    async fn subscribe(&self, topic: &str, handler: EventHandler) -> Result<()>;
}
```

实现：

1. `NatsEventBus`（默认）
2. `WsEventBridge`（向外部 WebSocket 客户端转发）

## 5. 配置模型（新增 transport）

```toml
[transport.grpc]
enabled = true
listen = "0.0.0.0:50051"
reflection = true

[transport.rpc]
enabled = true
default = "grpc" # grpc | nats
timeout_ms = 3000

[transport.nats]
enabled = true
url = "nats://127.0.0.1:4222"
jetstream = false

[transport.websocket]
enabled = true
path = "/ws"
max_connections = 10000
heartbeat_seconds = 20

[transport.p2p]
enabled = false
listen = "/ip4/0.0.0.0/tcp/7001"
bootstrap = []
```

## 6. Cargo Features（新增）

```toml
[features]
transport-grpc = ["dep:tonic", "dep:prost"]
transport-rpc = []
transport-nats = ["dep:async-nats"]
transport-ws = []
transport-p2p = ["dep:libp2p"] # experimental
```

规则：

1. `transport-rpc` 仅定义抽象，不绑定具体协议。
2. `rpc.default=grpc` 时需要 `transport-grpc`。
3. `rpc.default=nats` 时需要 `transport-nats`。

## 7. 对外接口策略（Edge）

## 7.1 RESTful

1. 资源模型、幂等、缓存语义由 REST 默认规范支持。
2. 作为开放 API 首选，兼容 Web 与移动端。

## 7.2 RPC

1. 对内对外都可用，但默认主用于内网 BFF/高频调用。
2. 版本管理采用 proto package + service version。

## 7.3 WebSocket

1. 实时推送与双向通信。
2. 推荐和 NATS 结合：后端发布事件，网关 fanout 给连接会话。

## 7.4 P2P

1. 用于边缘节点直连或去中心化同步场景。
2. 默认 `experimental`，需要显式启用。

## 8. Service-to-Service 标准模式

建议内置两种模板：

1. `request-response`：gRPC（主）+ NATS request/reply（轻量）
2. `event-driven`：NATS pub/sub（主）+ WebSocket bridge（可选）

约束：

1. 同步调用必须定义超时、重试、熔断。
2. 异步事件必须定义 topic 规范、幂等 key、死信策略（后续 JetStream）。

## 9. 性能关键点

1. gRPC 使用 HTTP/2 多路复用，减少连接开销。
2. NATS 客户端复用连接并限制订阅回调阻塞。
3. WebSocket 采用分层 broadcaster，避免全局锁热点。
4. P2P 默认关闭，避免引入额外运行时开销。

## 10. 首批落地范围（对齐你的确认）

1. 配置和 feature：`transport-grpc`、`transport-nats`、`transport-ws` 先落。
2. 实现能力：
   - service-to-service: gRPC + NATS RPC + NATS event
   - edge: REST + gRPC + WebSocket
3. `transport-p2p` 只定义抽象与配置，占位 experimental，不进入首批实现。

## 11. 与现有路线的合并

1. 数据集成首批：`postgres + redis + mongodb + nats + qdrant + neo4j`
2. 协议首批：`grpc + nats + websocket`
3. 统一由 `cargo hwhkit init` 生成模板并自动开启所需 features。
