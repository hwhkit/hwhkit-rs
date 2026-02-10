# HwhKit 目录结构与模板结构三方案

日期：2026-02-09  
索引：04  
状态：建议评审

## 1. 目标

围绕你的要求（极致性能、最小可变面、`cargo hwhkit init`、多协议与多中间件），给出三套结构方案：

1. 当前 HwhKit 仓库如何组织。
2. `init` 生成的业务项目模板如何组织。

## 2. 方案 A：单仓单包演进（最小改造）

## 2.1 HwhKit 仓库结构

```text
hwhkit-rs/
  Cargo.toml
  src/
    lib.rs
    bootstrap/
      mod.rs
      runtime.rs
    config/
      mod.rs
      loader.rs
      validate.rs
      source/
        file.rs
        env.rs
        remote.rs
    observability/
      mod.rs
      logging.rs
      tracing.rs
    integration/
      mod.rs
      postgres.rs
      redis.rs
      mongodb.rs
      nats.rs
      qdrant.rs
      neo4j.rs
    transport/
      mod.rs
      grpc.rs
      rpc.rs
      websocket.rs
      p2p.rs
    web/
      mod.rs
      middleware.rs
      health.rs
      error_response.rs
  src/bin/
    cargo-hwhkit.rs
  templates/
    service-api/
    service-realtime/
  examples/
  tests/
```

## 2.2 `cargo hwhkit init` 模板结构

```text
my-service/
  Cargo.toml
  src/
    main.rs
    app.rs
    routes/
      mod.rs
      health.rs
      api.rs
    domain/
      mod.rs
      user_service.rs
  config/
    default.toml
    dev.toml
    prod.toml
  proto/
  tests/
    smoke.rs
```

## 2.3 优缺点

1. 优点：改造成本最低，最快落地。
2. 缺点：库与 CLI 耦合高；feature 增多后编译边界不清晰。
3. 适用：你要在 1-2 周内快速交付首版。

## 3. 方案 B：Workspace 分层双核心（推荐）

## 3.1 HwhKit 仓库结构

```text
hwhkit-rs/
  Cargo.toml                    # workspace root
  crates/
    hwhkit/                     # facade crate，对外唯一依赖入口
      Cargo.toml
      src/lib.rs
    hwhkit-core/                # 配置/启动/观测/错误/health
      Cargo.toml
      src/
        bootstrap/
        config/
        observability/
        web/
    hwhkit-integrations/        # postgres/redis/mongodb/nats/qdrant/neo4j
      Cargo.toml
      src/
        provider/
        context/
    hwhkit-transport/           # grpc/rpc/ws/p2p
      Cargo.toml
      src/
        grpc/
        rpc/
        websocket/
        p2p/
    cargo-hwhkit/               # cargo hwhkit init
      Cargo.toml
      src/main.rs
  templates/
    minimal-api/
    api-grpc/
    realtime-event/
  examples/
  tests/
  doc/
```

## 3.2 `cargo hwhkit init` 模板结构

```text
my-service/
  Cargo.toml
  src/
    main.rs                     # hwhkit::bootstrap::run(App)
    app.rs                      # Application trait impl
    state.rs
    routes/
      mod.rs
      rest.rs
      ws.rs
    rpc/
      mod.rs
      client.rs
    domain/
      mod.rs
      service.rs
    infra/
      mod.rs
      repository.rs
      event_bus.rs
  config/
    default.toml
    dev.toml
    prod.toml
  proto/
    app/v1/service.proto
  scripts/
    gen-proto.sh
  tests/
    smoke.rs
    contract_rest.rs
    contract_grpc.rs
```

## 3.3 优缺点

1. 优点：边界清晰，编译更快，CLI 与库分离，最符合长期演进。
2. 缺点：初期重构工作量中等。
3. 适用：你要把 HwhKit 做成长期维护的工程化平台。

## 4. 方案 C：平台化插件生态（最大扩展）

## 4.1 HwhKit 仓库结构

```text
hwhkit-rs/
  Cargo.toml
  crates/
    hwhkit/                     # facade
    hwhkit-core/
    hwhkit-runtime/
    hwhkit-observability/
    hwhkit-config-center/       # 自建配置中心 client + watch
    hwhkit-transport/
    hwhkit-plugin-api/          # 外部插件标准接口
    hwhkit-plugin-sdk/          # 开发插件的辅助库
    hwhkit-integrations-oss/    # 开源集成集合
    cargo-hwhkit/
  plugins/
    postgres/
    redis/
    mongodb/
    nats/
    qdrant/
    neo4j/
  templates/
    enterprise-mesh/
    edge-gateway/
    worker-consumer/
```

## 4.2 `cargo hwhkit init` 模板结构

```text
my-platform-service/
  Cargo.toml
  src/
    main.rs
    app.rs
    module/
      user/
      order/
    transport/
      rest/
      grpc/
      ws/
    workflow/
      saga/
      outbox/
  config/
    default.toml
    dev.toml
    prod.toml
  deploy/
    docker/
    k8s/
  proto/
  tests/
  benches/
```

## 4.3 优缺点

1. 优点：扩展性最强，插件生态最完整。
2. 缺点：复杂度高，维护成本高，对团队工程纪律要求高。
3. 适用：多团队平台化、需要插件市场或私有扩展生态。

## 5. 推荐结论

推荐采用方案 B（Workspace 分层双核心）。

原因：

1. 对你当前阶段最平衡：能快速推进，同时不会把后续演进堵死。
2. 和你确认的首批目标完全对齐：`cargo hwhkit init`、分层配置、首批 integrations、grpc/nats/ws。
3. 未来可平滑升级到方案 C（把 `hwhkit-integrations` 再拆成插件包）。

## 6. 迁移建议（从当前仓库到方案 B）

1. 第一步：建立 workspace 与 `cargo-hwhkit` crate，保留现有 `src/*` 行为不变。
2. 第二步：把配置/观测/启动迁移到 `hwhkit-core`。
3. 第三步：把首批中间件迁移到 `hwhkit-integrations`。
4. 第四步：把 grpc/rpc/ws 迁移到 `hwhkit-transport`。
5. 第五步：`hwhkit` facade 统一 re-export，对用户保持单依赖入口。
