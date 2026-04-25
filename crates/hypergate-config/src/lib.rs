//! 动态配置快照、加载和重载协调。

#![deny(missing_docs)]

mod extension;
mod manager;
mod schema;

pub use extension::{
    ConfigLoader, ConfigValidator, ConfigValidatorChain, DefaultConfigValidator, StaticConfigLoader,
};
pub use manager::ConfigManager;
pub use schema::{
    DrainConfig, ReloadTrigger, RuntimeConfig, ServerConfig, VersionConfig, WatchConfig,
};
