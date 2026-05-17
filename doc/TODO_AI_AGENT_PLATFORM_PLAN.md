# HwhKit AI Agent Platform TODO

本文件定义 HwhKit 面向 AI Agent Service / AI Company 的能力建设路线。

核心目标：

1. 能作为 AI 服务平台的稳定底座。
2. 能同时承载 API、workflow、agent runtime、tool calling、memory、retrieval、multi-service mesh。
3. 能支持从单服务到公司级内部平台的演进。

状态标记：

- `[x]` 已完成
- `[-]` 进行中
- `[ ]` 未开始

## Phase 1: AI Runtime Foundations

- [ ] 定义 `AgentModule` 抽象
- [ ] 定义 `ToolRegistry` 抽象
- [ ] 定义 `PromptTemplate` / `WorkflowTemplate` 抽象
- [ ] 定义 `SessionContext` / `ConversationState` 抽象
- [ ] 定义 `MemoryStore` 抽象

验收标准：

- Agent、Tool、Workflow、Memory 都能以模块方式挂载到 runtime

## Phase 2: LLM Provider Layer

- [ ] OpenAI provider
- [ ] Anthropic provider
- [ ] OpenRouter / compatible provider
- [ ] 本地模型 provider（Ollama / vLLM / LM Studio 类）
- [ ] 统一 `ChatModel` / `EmbeddingModel` / `ReasoningModel` 接口

验收标准：

- 业务层不直接依赖单一模型 SDK
- 模型路由可配置

## Phase 3: Retrieval And Knowledge

- [ ] 向量检索主链路：Qdrant
- [ ] 向量检索主链路：Milvus
- [ ] 文档切分与索引流水线
- [ ] Embedding 任务队列
- [ ] 检索缓存与重建策略

验收标准：

- AI 服务可以直接挂接 retrieval pipeline

## Phase 4: Agent Messaging And Orchestration

- [ ] 任务总线抽象
- [ ] Agent-to-Agent messaging
- [ ] RPC + event-driven 混合编排
- [ ] 长任务状态机
- [ ] checkpoint / resume 机制

验收标准：

- 支持同步推理、异步任务、事件驱动 agent 协作

## Phase 5: Memory And State

- [ ] 短期会话状态
- [ ] 长期记忆存储
- [ ] 用户画像/组织知识抽象
- [ ] 审计日志与可追溯性
- [ ] state compaction / summarization

验收标准：

- 支持 AI 公司级别的多租户会话与知识沉淀

## Phase 6: Tooling And Enterprise Integration

- [ ] HTTP tools
- [ ] DB tools
- [ ] Search tools
- [ ] Queue / workflow tools
- [ ] Internal service tools
- [ ] Auth / policy / approval tools

验收标准：

- Agent 能安全地接入公司内部系统

## Phase 7: Platform Concerns

- [ ] 多租户
- [ ] 配额与限流
- [ ] 成本跟踪
- [ ] 观测：trace / metrics / structured logs
- [ ] 权限模型
- [ ] secrets 管理

验收标准：

- 支持平台化运营，而非只适合 demo

## Phase 8: AI Company Operating System

- [ ] API Gateway for AI services
- [ ] Workflow orchestration service
- [ ] Agent runtime service
- [ ] Knowledge ingestion service
- [ ] Retrieval service
- [ ] Tool execution sandbox service
- [ ] Billing / usage service
- [ ] Admin console / ops surface

验收标准：

- HwhKit 能承载完整 AI 公司内部平台的服务拆分与缝合

## Immediate Next Actions

- [ ] 1. 先把资源与 mesh 主线做稳
- [ ] 2. 增加统一模型 provider 抽象
- [ ] 3. 增加 memory / retrieval 基础接口
- [ ] 4. 设计 AgentModule / ToolRegistry
- [ ] 5. 增加 AI service demo
