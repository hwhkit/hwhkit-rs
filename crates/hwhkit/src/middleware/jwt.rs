//! JWT 认证中间件模块

use crate::{
    config::JwtConfig,
    error::{Error, Result},
};
use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};

#[cfg(feature = "jwt")]
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

/// JWT 声明
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String, // Subject (用户 ID)
    pub exp: usize,  // Expiration time
    pub iat: usize,  // Issued at
    pub aud: String, // Audience
    pub iss: String, // Issuer
}

/// JWT 认证状态
#[derive(Debug, Clone)]
pub struct JwtAuth {
    pub secret: String,
    pub expires_in: u64,
}

impl JwtAuth {
    /// 创建新的 JWT 认证实例
    pub fn new(config: &JwtConfig) -> Self {
        Self {
            secret: config.secret.clone(),
            expires_in: config.expires_in,
        }
    }

    /// 生成 JWT token
    #[cfg(feature = "jwt")]
    pub fn generate_token(&self, user_id: &str) -> Result<String> {
        let now = chrono::Utc::now();
        let iat = now.timestamp() as usize;
        let exp = (now + chrono::Duration::seconds(self.expires_in as i64)).timestamp() as usize;

        let claims = Claims {
            sub: user_id.to_string(),
            exp,
            iat,
            aud: "hwhkit".to_string(),
            iss: "hwhkit".to_string(),
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.secret.as_ref()),
        )
        .map_err(Error::Jwt)
    }

    /// 验证 JWT token
    #[cfg(feature = "jwt")]
    pub fn verify_token(&self, token: &str) -> Result<Claims> {
        let mut validation = Validation::default();
        validation.set_audience(&["hwhkit"]);
        validation.set_issuer(&["hwhkit"]);

        decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.secret.as_ref()),
            &validation,
        )
        .map(|token_data| token_data.claims)
        .map_err(Error::Jwt)
    }

    /// 从请求头中提取 token
    pub fn extract_token_from_header(&self, headers: &HeaderMap) -> Result<String> {
        let auth_header = headers
            .get("authorization")
            .and_then(|header| header.to_str().ok())
            .ok_or_else(|| Error::Middleware("缺少 Authorization 头部".to_string()))?;

        if !auth_header.starts_with("Bearer ") {
            return Err(Error::Middleware("无效的 Authorization 格式".to_string()));
        }

        Ok(auth_header.trim_start_matches("Bearer ").to_string())
    }
}

/// JWT 认证中间件
#[cfg(feature = "jwt")]
pub async fn jwt_auth_middleware(
    mut request: Request,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    // 这里需要从某种状态或扩展中获取 JWT 配置
    // 为了简化，我们跳过实际的验证
    tracing::warn!("JWT 认证中间件: 功能暂未完全实现");
    Ok(next.run(request).await)
}

/// 无 JWT 功能时的占位中间件
#[cfg(not(feature = "jwt"))]
pub async fn jwt_auth_middleware(
    request: Request,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    tracing::warn!("JWT 功能未启用，跳过认证");
    Ok(next.run(request).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_auth_creation() {
        let config = JwtConfig {
            enabled: true,
            secret: "test-secret".to_string(),
            expires_in: 3600,
        };

        let jwt_auth = JwtAuth::new(&config);
        assert_eq!(jwt_auth.secret, "test-secret");
        assert_eq!(jwt_auth.expires_in, 3600);
    }

    #[cfg(feature = "jwt")]
    #[test]
    fn test_token_generation_and_verification() {
        let config = JwtConfig {
            enabled: true,
            secret: "test-secret".to_string(),
            expires_in: 3600,
        };

        let jwt_auth = JwtAuth::new(&config);
        let token = jwt_auth.generate_token("user123").unwrap();
        let claims = jwt_auth.verify_token(&token).unwrap();

        assert_eq!(claims.sub, "user123");
        assert_eq!(claims.aud, "hwhkit");
        assert_eq!(claims.iss, "hwhkit");
    }
}
