//! Demo version app 配置示例。
//!
//! 这里展示业务配置如何接入 `hypergate-config` 的快照管理。简单配置
//! 使用 `ConfigManager::static_config`,不需要开发者手写 loader 或
//! validator 壳类型。
//!
//! @author sky

use std::sync::Arc;

use hypergate_app::render_panel;
use hypergate_config::ConfigManager;
use hypergate_core::{HyperError, HyperResult};

/// Demo 配置管理器句柄。
pub(crate) type DemoConfigHandle = Arc<ConfigManager<DemoConfig>>;

/// Demo 业务配置。
#[derive(Clone)]
pub(crate) struct DemoConfig {
    /// 示例应用名称。
    pub(crate) name: String,
    /// 默认响应前缀。
    pub(crate) greeting: String,
}

/// 创建 demo 配置管理器。
pub(crate) fn demo_config_handle() -> DemoConfigHandle {
    Arc::new(ConfigManager::static_config(
        DemoConfig {
            name: "demo".to_owned(),
            greeting: "HyperGate".to_owned(),
        },
        validate_demo_config,
    ))
}

/// 渲染 demo 配置面板。
pub(crate) fn demo_config_panel(revision: u64, config: &DemoConfig) -> String {
    render_panel(
        "Demo Config",
        vec![
            ("revision".to_owned(), revision.to_string()),
            ("name".to_owned(), config.name.clone()),
            ("greeting".to_owned(), config.greeting.clone()),
        ],
        Vec::new(),
    )
}

/// 校验 demo 运行需要的最小业务配置。
fn validate_demo_config(config: &DemoConfig) -> HyperResult<()> {
    if config.name.trim().is_empty() {
        return Err(HyperError::new("demo config name is empty"));
    }
    if config.greeting.trim().is_empty() {
        return Err(HyperError::new("demo config greeting is empty"));
    }
    Ok(())
}
