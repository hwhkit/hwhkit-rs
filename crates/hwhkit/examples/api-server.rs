//! 前后端分离架构示例
//!
//! 这个示例展示了如何使用 HwhKit 创建一个 API 服务器

use axum::{
    extract::{Json, Path},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use hwhkit::{Deserialize, Serialize, WebServerBuilder};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
    email: String,
}

#[derive(Serialize)]
struct ApiResponse<T> {
    success: bool,
    data: T,
    message: String,
}

impl<T> ApiResponse<T> {
    fn success(data: T) -> Self {
        Self {
            success: true,
            data,
            message: "操作成功".to_string(),
        }
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    success: bool,
    error: String,
    code: u16,
}

impl ErrorResponse {
    fn new(code: u16, error: String) -> Self {
        Self {
            success: false,
            error,
            code,
        }
    }
}

// 模拟数据库
type UserStore = std::sync::Arc<std::sync::Mutex<HashMap<u64, User>>>;

// API 路由处理器
async fn get_users() -> impl IntoResponse {
    let users = vec![
        User {
            id: 1,
            name: "张三".to_string(),
            email: "zhangsan@example.com".to_string(),
        },
        User {
            id: 2,
            name: "李四".to_string(),
            email: "lisi@example.com".to_string(),
        },
    ];

    Json(ApiResponse::success(users))
}

async fn get_user(Path(id): Path<u64>) -> impl IntoResponse {
    if id == 1 {
        let user = User {
            id: 1,
            name: "张三".to_string(),
            email: "zhangsan@example.com".to_string(),
        };
        (StatusCode::OK, Json(ApiResponse::success(user))).into_response()
    } else {
        let error = ErrorResponse::new(404, "用户不存在".to_string());
        (StatusCode::NOT_FOUND, Json(error)).into_response()
    }
}

async fn create_user(Json(user): Json<User>) -> impl IntoResponse {
    // 简单验证
    if user.name.is_empty() || user.email.is_empty() {
        let error = ErrorResponse::new(400, "用户名和邮箱不能为空".to_string());
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // 在实际应用中，这里会保存到数据库
    (StatusCode::CREATED, Json(ApiResponse::success(user))).into_response()
}

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "HwhKit API Server"
    }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 构建 API 路由
    let api_routes = Router::new()
        .route("/users", get(get_users).post(create_user))
        .route("/users/:id", get(get_user))
        .route("/health", get(health_check));

    // 构建应用路由
    let app_routes = Router::new()
        .nest("/api/v1", api_routes)
        .route("/", get(|| async { "HwhKit API Server is running!" }));

    // 创建服务器
    let server = WebServerBuilder::new()
        .config_from_file("examples/api-config.toml")
        .routes(app_routes)
        .build()
        .await?;

    println!("🚀 API 服务器启动中...");
    println!("📖 API 文档: http://localhost:3000/");
    println!("💾 健康检查: http://localhost:3000/api/v1/health");
    println!("👥 用户列表: http://localhost:3000/api/v1/users");

    server.serve().await?;

    Ok(())
}
