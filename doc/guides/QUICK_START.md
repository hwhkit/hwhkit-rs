# HwhKit 快速开始指南

这个指南将帮助你在 5 分钟内使用 HwhKit 构建你的第一个 Web 服务。

## 🎯 目标

通过这个指南，你将学会：
- 安装和设置 HwhKit
- 创建一个基本的 API 服务器
- 创建一个全栈 Web 应用
- 使用配置文件管理中间件

## 📦 准备工作

确保你的系统上安装了：
- Rust 1.70.0 或更高版本
- Cargo（通常随 Rust 一起安装）

检查版本：
```bash
rustc --version
cargo --version
```

## 🚀 第一步：创建新项目

```bash
cargo new my-web-app
cd my-web-app
```

## 📝 第二步：添加依赖

编辑 `Cargo.toml`：

```toml
[package]
name = "my-web-app"
version = "0.1.0"
edition = "2021"

[dependencies]
hwhkit = "0.1.0"
tokio = { version = "1.0", features = ["full"] }
serde = { version = "1.0", features = ["derive"] }
```

## 🔧 第三步：构建 API 服务器

创建 `src/main.rs`：

```rust
use hwhkit::{WebServerBuilder, get, Json, Serialize, Deserialize};

#[derive(Serialize)]
struct ApiResponse {
    message: String,
    timestamp: String,
}

#[derive(Deserialize)]
struct CreateUserRequest {
    name: String,
    email: String,
}

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
    email: String,
}

// 路由处理函数
async fn hello() -> Json<ApiResponse> {
    Json(ApiResponse {
        message: "Hello from HwhKit!".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}

async fn get_users() -> Json<Vec<User>> {
    let users = vec![
        User { id: 1, name: "Alice".to_string(), email: "alice@example.com".to_string() },
        User { id: 2, name: "Bob".to_string(), email: "bob@example.com".to_string() },
    ];
    Json(users)
}

async fn create_user(Json(request): Json<CreateUserRequest>) -> Json<User> {
    // 在实际应用中，这里会保存到数据库
    let user = User {
        id: 3, // 模拟生成的 ID
        name: request.name,
        email: request.email,
    };
    Json(user)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 构建路由
    let app = hwhkit::Router::new()
        .route("/", get(hello))
        .route("/users", get(get_users).post(create_user));

    // 创建服务器
    let server = WebServerBuilder::new()
        .listen("0.0.0.0", 3000)
        .cors(vec!["*".to_string()]) // 允许所有来源（开发环境）
        .routes(app)
        .build()
        .await?;

    println!("🚀 服务器启动在 http://localhost:3000");
    println!("📖 API 端点:");
    println!("  GET  /       - 欢迎消息");
    println!("  GET  /users  - 获取用户列表");
    println!("  POST /users  - 创建新用户");

    server.serve().await?;
    Ok(())
}
```

## ⚡ 第四步：运行服务器

```bash
cargo run
```

访问你的 API：

```bash
# 获取欢迎消息
curl http://localhost:3000/

# 获取用户列表
curl http://localhost:3000/users

# 创建新用户
curl -X POST http://localhost:3000/users \
  -H "Content-Type: application/json" \
  -d '{"name": "Charlie", "email": "charlie@example.com"}'
```

## 🏗️ 第五步：使用配置文件

创建 `config.toml`：

```toml
[server]
host = "0.0.0.0"
port = 3000
architecture = "api"

[middleware.cors]
enabled = true
origins = ["http://localhost:3000", "https://yourdomain.com"]
methods = ["GET", "POST", "PUT", "DELETE"]
headers = ["Content-Type", "Authorization"]

[middleware.jwt]
enabled = false  # 暂时禁用，后面可以启用
secret = "your-super-secure-secret"
expires_in = 3600

[middleware.static_files]
enabled = true
dir = "public"
prefix = "/static"

[middleware.logging]
level = "info"
requests = true

[middleware.custom]
app_name = "My Web App"
version = "1.0.0"
```

更新 `src/main.rs` 使用配置文件：

```rust
use hwhkit::{WebServerBuilder, get, post, Json, Serialize, Deserialize};

// ... 保持之前的结构体定义 ...

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 构建路由
    let app = hwhkit::Router::new()
        .route("/", get(hello))
        .route("/users", get(get_users).post(create_user));

    // 使用配置文件创建服务器
    let server = WebServerBuilder::new()
        .config_from_file("config.toml")  // 从配置文件加载
        .routes(app)
        .build()
        .await?;

    println!("🚀 服务器启动成功！");
    server.serve().await?;
    Ok(())
}
```

## 🎨 第六步：添加静态文件（可选）

创建 `public` 目录和一个简单的 HTML 文件：

```bash
mkdir public
```

创建 `public/index.html`：

```html
<!DOCTYPE html>
<html>
<head>
    <title>My Web App</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 40px; }
        .container { max-width: 600px; margin: 0 auto; }
        .btn { background: #007bff; color: white; padding: 10px 20px; border: none; border-radius: 4px; cursor: pointer; }
    </style>
</head>
<body>
    <div class="container">
        <h1>欢迎使用 HwhKit!</h1>
        <p>你的 Web 服务器正在运行中。</p>
        
        <h2>API 测试</h2>
        <button class="btn" onclick="testApi()">测试 API</button>
        <div id="result"></div>
        
        <script>
            async function testApi() {
                try {
                    const response = await fetch('/users');
                    const users = await response.json();
                    document.getElementById('result').innerHTML = 
                        '<h3>用户列表:</h3><pre>' + JSON.stringify(users, null, 2) + '</pre>';
                } catch (error) {
                    document.getElementById('result').innerHTML = 
                        '<p style="color: red;">错误: ' + error.message + '</p>';
                }
            }
        </script>
    </div>
</body>
</html>
```

现在你可以访问 `http://localhost:3000/static/index.html` 查看前端页面！

## 🌟 下一步

恭喜！你已经成功创建了第一个 HwhKit 应用。接下来你可以：

1. **探索更多功能**：
   - 启用 JWT 认证
   - 添加数据库集成
   - 使用模板引擎

2. **查看示例**：
   ```bash
   git clone https://github.com/yourusername/hwhkit.git
   cd hwhkit
   cargo run --example api-server
   cargo run --example full-server
   ```

3. **阅读完整文档**：
   - [README.md](README.md)
   - [API 文档](https://docs.rs/hwhkit)

4. **加入社区**：
   - [GitHub Issues](https://github.com/yourusername/hwhkit/issues)
   - [Discussions](https://github.com/yourusername/hwhkit/discussions)

## 💡 常见问题

**Q: 如何启用 HTTPS？**
A: HwhKit 专注于应用层，建议在生产环境中使用反向代理（如 Nginx）来处理 HTTPS。

**Q: 如何添加数据库？**
A: HwhKit 与数据库无关，你可以使用任何 Rust 数据库库（如 sqlx、diesel 等）。

**Q: 如何部署到生产环境？**
A: 编译你的应用（`cargo build --release`），然后将二进制文件和配置文件部署到服务器。

## 🎉 完成！

你现在已经掌握了 HwhKit 的基础用法。开始构建你的下一个伟大的 Web 应用吧！

---

如果遇到问题，请查看 [故障排除指南](README.md#troubleshooting) 或 [提交 Issue](https://github.com/yourusername/hwhkit/issues)。