# HwhKit Testing And Quality TODO

本文件定义 HwhKit 的全面测试与质量保证执行清单。

目标：

1. 保证 workspace 每个 crate 至少有基础单测与契约测试。
2. 保证 facade、config、runtime、transport、providers 的主链路可验证。
3. 保证新版本在 AI Agent Service / AI Company 场景下具备可依赖性。

状态标记：

- `[x]` 已完成
- `[-]` 进行中
- `[ ]` 未开始

## Phase 1: Build And Static Checks

- [x] `cargo check --workspace --all-targets --all-features`
- [x] `cargo test --workspace --all-targets`
- [x] `cargo test --workspace --all-features`
- [x] `cargo fmt --all --check`
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`

验收标准：

- workspace 在默认特性和全特性下均可编译
- clippy 无 warning
- fmt 检查通过

## Phase 2: Config Layer

- [x] layered config 合并测试
- [ ] env override 测试
- [x] remote patch policy 测试
- [x] `resources.*` 校验测试
- [x] `mesh.services.*` 校验测试
- [ ] feature/config mismatch 测试

验收标准：

- 非法配置能给出稳定、清晰的错误信息
- 关键配置路径的单测覆盖齐全

## Phase 3: Core Runtime

- [ ] `ResourceRegistry` 注册/读取/覆盖测试
- [ ] `ServiceRegistry` local/remote/auto 测试
- [ ] `AppContext` 资源和服务访问测试
- [ ] bootstrap 初始化顺序测试
- [x] required integration 失败时中止测试
- [x] optional integration 失败时 degraded 测试

验收标准：

- 本地服务优先、远程回退行为稳定
- 初始化失败与降级行为符合设计

## Phase 4: Transport And Mesh

- [x] `AdapterRegistry` 注册与分发测试
- [x] `MeshClient` 路由解析测试
- [ ] `MeshClient` 对 `rpc/http/nats/ws/socket` 的协议映射测试
- [x] loopback adapter 测试
- [x] request metadata 透传测试
- [x] timeout/serializer metadata 测试

验收标准：

- 所有已声明协议都能通过统一入口分发
- 元数据写入规则稳定

## Phase 5: Providers

- [x] postgres provider 配置校验测试
- [x] redis provider 配置校验测试
- [x] mongodb provider 配置校验测试
- [x] nats provider 配置校验测试
- [x] qdrant provider 配置校验测试
- [x] neo4j provider 配置校验测试
- [x] provider 注册到 `ResourceRegistry` 测试

验收标准：

- 每个 provider 至少验证：
  - 配置合法性
  - 初始化成功路径
  - 资源注册成功

## Phase 6: Facade And Compatibility

- [x] facade re-export 编译测试
- [x] `run_v2` 示例编译测试
- [x] `WebServerBuilder` 兼容性测试
- [x] v1/v2 API 并存测试

验收标准：

- facade 仍是唯一推荐入口
- 兼容层不阻塞 v3 主线

## Phase 7: End-To-End

- [x] HTTP healthz demo 测试
- [x] local service mesh demo 测试
- [x] remote mesh loopback demo 测试
- [x] 配置切换协议不改业务代码测试
- [x] CLI init 模板 smoke 测试

验收标准：

- 至少一个 demo 覆盖配置驱动的服务缝合

## Phase 8: Reliability

- [ ] panic safety review
- [ ] shutdown 生命周期测试
- [ ] 并发调用测试
- [ ] 大量注册资源/服务测试
- [ ] 错误信息稳定性测试

验收标准：

- 关键运行时对象在并发下无明显行为错误

## Execution Order

- [x] 1. 先跑 workspace 编译与测试，修复阻塞项
- [-] 2. 补足 config/core/transport 的单元测试
- [x] 3. 补足 provider 的资源注册测试
- [x] 4. 增加 facade 与 CLI smoke 测试
- [x] 5. 增加端到端场景测试
- [ ] 6. 打通 clippy/fmt/check 的 CI 标准
