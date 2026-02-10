# 贡献指南

感谢您对 HwhKit 项目的关注！我们欢迎任何形式的贡献，包括但不限于：

- 🐛 报告 Bug
- 💡 提出新功能建议
- 📝 改进文档
- 🔧 提交代码修复
- ✨ 实现新功能

## 🚀 快速开始

### 环境要求

- Rust 1.70.0 或更高版本
- Git

### 设置开发环境

1. **Fork 并克隆仓库**

```bash
git clone https://github.com/yourusername/hwhkit.git
cd hwhkit
```

2. **安装依赖**

```bash
cargo build
```

3. **运行测试确保一切正常**

```bash
cargo test
```

4. **运行示例程序**

```bash
# API 服务器示例
cargo run --example api-server

# 全栈服务器示例
cargo run --example full-server
```

## 📋 开发流程

### 1. 创建分支

为你的功能或修复创建一个新分支：

```bash
git checkout -b feature/your-feature-name
# 或
git checkout -b fix/issue-description
```

分支命名约定：
- `feature/` - 新功能
- `fix/` - Bug 修复
- `docs/` - 文档更新
- `refactor/` - 代码重构
- `test/` - 测试相关

### 2. 进行开发

#### 代码风格

- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 检查代码质量
- 遵循 Rust 官方代码风格指南

#### 提交消息格式

使用清晰的提交消息：

```
type(scope): 简短描述

详细描述（可选）

Fixes #issue_number（如果适用）
```

类型：
- `feat`: 新功能
- `fix`: Bug 修复
- `docs`: 文档更新
- `style`: 代码格式化
- `refactor`: 代码重构
- `test`: 测试相关
- `chore`: 构建流程或辅助工具的变动

示例：
```
feat(middleware): 添加请求限流中间件

实现基于 Token bucket 算法的请求限流中间件，
支持按 IP 和用户进行限流。

Fixes #123
```

### 3. 测试

确保所有测试都通过：

```bash
# 运行所有测试
cargo test

# 运行特定特性的测试
cargo test --features "templates,jwt"

# 运行集成测试
cargo test --test integration

# 检查代码覆盖率（需要安装 tarpaulin）
cargo tarpaulin --out Html
```

添加新功能时，请确保：
- 编写单元测试
- 编写集成测试（如果适用）
- 更新相关文档

### 4. 文档

如果你的更改影响了公共 API：

- 更新 API 文档注释
- 运行 `cargo doc` 确保文档正确生成
- 更新 README.md（如果需要）
- 添加或更新示例

### 5. 提交 Pull Request

1. **推送分支到你的 Fork**

```bash
git push origin feature/your-feature-name
```

2. **创建 Pull Request**

在 GitHub 上创建 Pull Request，请包含：

- 清晰的标题和描述
- 更改的原因和内容
- 相关的 Issue 编号
- 测试结果
- 截图（如果是 UI 相关）

3. **Pull Request 模板**

```markdown
## 描述
简要描述这个 PR 的目的和更改内容。

## 更改类型
- [ ] Bug 修复
- [ ] 新功能
- [ ] 重大更改
- [ ] 文档更新
- [ ] 性能改进
- [ ] 其他（请说明）

## 测试
- [ ] 新增了单元测试
- [ ] 新增了集成测试
- [ ] 所有现有测试通过
- [ ] 手动测试通过

## 检查清单
- [ ] 代码遵循项目风格指南
- [ ] 进行了自我代码审查
- [ ] 代码有适当的注释
- [ ] 相应的文档已更新
- [ ] 更改不产生新的警告
- [ ] 添加了测试来证明修复有效或新功能有效

## 相关 Issue
Fixes #(issue)
```

## 🐛 报告 Bug

使用 [GitHub Issues](https://github.com/yourusername/hwhkit/issues) 报告 Bug。

请包含：

- Bug 的简要描述
- 重现步骤
- 期望的行为
- 实际的行为
- 环境信息（操作系统、Rust 版本等）
- 相关的代码片段或错误信息

### Bug 报告模板

```markdown
**描述 Bug**
简要清晰地描述 Bug 是什么。

**重现步骤**
重现这个行为的步骤：
1. 进入 '...'
2. 点击 '....'
3. 滚动到 '....'
4. 看到错误

**期望行为**
简要清晰地描述你期望发生什么。

**实际行为**
简要清晰地描述实际发生了什么。

**截图**
如果适用，添加截图来帮助解释你的问题。

**环境信息：**
 - 操作系统：[例如 macOS 12.0]
 - Rust 版本：[例如 1.70.0]
 - HwhKit 版本：[例如 0.1.0]

**额外信息**
添加任何其他有关问题的信息。
```

## 💡 功能请求

我们欢迎新功能建议！请通过 [GitHub Issues](https://github.com/yourusername/hwhkit/issues) 提交功能请求。

请包含：

- 功能的清晰描述
- 使用场景和动机
- 可能的实现方案
- 是否愿意贡献代码

## 📝 文档贡献

文档改进同样重要！你可以：

- 修复文档中的错误
- 改进现有文档的清晰度
- 添加使用示例
- 翻译文档

## 🏗️ 架构概述

了解项目结构有助于贡献：

```
src/
├── lib.rs              # 库入口点
├── builder.rs          # Web 服务器构建器
├── config.rs           # 配置管理
├── error.rs            # 错误处理
├── server.rs           # Web 服务器实现
├── middleware/         # 中间件模块
│   ├── mod.rs
│   ├── cors.rs         # CORS 中间件
│   ├── jwt.rs          # JWT 认证
│   ├── logging.rs      # 日志中间件
│   └── static_files.rs # 静态文件服务
└── templates.rs        # 模板引擎（可选）

examples/               # 示例项目
├── api-server.rs       # API 服务器示例
├── full-server.rs      # 全栈服务器示例
├── api-config.toml     # API 配置示例
├── full-config.toml    # 全栈配置示例
└── static/             # 静态文件
    ├── style.css
    └── script.js

tests/                  # 测试文件
├── integration/        # 集成测试
└── unit/              # 单元测试
```

## 🔧 开发工具

推荐的开发工具和配置：

### VS Code 扩展

- rust-analyzer
- CodeLLDB
- Better TOML
- Error Lens

### 有用的 Cargo 工具

```bash
# 代码格式化
cargo install rustfmt

# 代码检查
cargo install clippy

# 测试覆盖率
cargo install cargo-tarpaulin

# 基准测试
cargo install cargo-criterion

# 文档生成
cargo doc --open
```

## 🤝 行为准则

我们致力于为每个人提供一个受欢迎和包容的环境。参与本项目时，请：

- 尊重不同观点和经历
- 优雅地接受建设性批评
- 专注于对社区最有利的事情
- 对其他社区成员表现出同理心

## ❓ 需要帮助？

如果你有任何问题：

- 查看 [README.md](README.md)
- 搜索现有的 [Issues](https://github.com/yourusername/hwhkit/issues)
- 创建新的 Issue
- 参与 [Discussions](https://github.com/yourusername/hwhkit/discussions)

## 🎉 感谢

感谢所有贡献者！你们的努力让 HwhKit 变得更好。

---

再次感谢你的贡献！🚀