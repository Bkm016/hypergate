//! Gateway 默认启动配置。正式配置加载器接入前,这里提供本地可运行示例。

use hypergate_config::{RuntimeConfig, VersionConfig};
use hypergate_core::VersionId;

/// 生成本地开发可直接运行的默认配置。
pub(crate) fn default_config() -> RuntimeConfig {
    let mut config = RuntimeConfig::minimal();
    insert_sample_version(&mut config, "v1", 9101);
    insert_sample_version(&mut config, "v2", 9102);
    config
}

/// 向默认配置追加一个本地 version app。
fn insert_sample_version(config: &mut RuntimeConfig, id: &str, port: u16) {
    let endpoint = format!("http://127.0.0.1:{port}");
    config.versions.insert(
        VersionId::new(id),
        VersionConfig {
            endpoint: endpoint.clone(),
            health: Some(format!("{endpoint}/health")),
        },
    );
}
