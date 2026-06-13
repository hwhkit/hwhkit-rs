//! Re-exports of the canonical config types from `hwhkit-config`.
//!
//! The schema lives in `hwhkit-config` so `AppConfig.integrations.llm`
//! is type-correct; this crate just consumes it. Two thin re-exports
//! make the names ergonomic at the call site.

pub use hwhkit_config::{
    LlmIntegrationConfig as LlmConfig, LlmProviderCredentials as ProviderCredentials,
    LlmProvidersConfig as ProvidersConfig,
};
