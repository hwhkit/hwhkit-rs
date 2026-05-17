use async_trait::async_trait;
use axum::Router;
use hwhkit_config::AppConfig;

use crate::{AppContext, Result};

#[async_trait]
pub trait Module: Send + Sync {
    fn name(&self) -> &'static str;

    async fn register_services(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<()> {
        Ok(())
    }

    async fn router(&self, _ctx: AppContext, _cfg: &AppConfig) -> Result<Router> {
        Ok(Router::new())
    }
}
