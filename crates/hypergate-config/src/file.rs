//! TOML 运行配置加载器。

use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hypergate_core::{ConfigRevision, HyperError, HyperResult, VersionId};
use ipnet::IpNet;
use serde::Deserialize;

use crate::{ConfigLoader, DrainConfig, RuntimeConfig, ServerConfig, VersionConfig};

/// 从固定 TOML 文件加载 Gateway 运行配置。
pub struct TomlConfigLoader {
    path: PathBuf,
}

impl TomlConfigLoader {
    /// 创建 TOML 配置加载器。
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 返回配置文件路径。
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigLoader<RuntimeConfig> for TomlConfigLoader {
    fn load(&self, next_revision: ConfigRevision) -> HyperResult<RuntimeConfig> {
        let source = fs::read_to_string(&self.path).map_err(|error| {
            HyperError::new(format!(
                "read config {} failed: {error}",
                self.path.display()
            ))
        })?;
        let file: FileConfig = toml::from_str(&source).map_err(|error| {
            HyperError::new(format!(
                "parse config {} failed: {error}",
                self.path.display()
            ))
        })?;
        file.into_runtime(next_revision)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    default_version: String,
    #[serde(default)]
    server: FileServerConfig,
    #[serde(default)]
    drain: FileDrainConfig,
    versions: HashMap<String, FileVersionConfig>,
}

impl FileConfig {
    fn into_runtime(self, revision: ConfigRevision) -> HyperResult<RuntimeConfig> {
        let mut server = ServerConfig::default();
        if let Some(value) = self.server.listen {
            server.listen = value;
        }
        if let Some(value) = self.server.max_connections {
            server.max_connections = value;
        }
        if let Some(value) = self.server.header_read_timeout_ms {
            server.header_read_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.server.version_connect_timeout_ms {
            server.version_connect_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.server.version_response_header_timeout_ms {
            server.version_response_header_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.server.version_health_timeout_ms {
            server.version_health_timeout = Duration::from_millis(value);
        }
        if let Some(value) = self.server.shutdown_timeout_seconds {
            server.shutdown_timeout = Duration::from_secs(value);
        }
        server.trusted_proxies = self
            .server
            .trusted_proxies
            .into_iter()
            .map(|value| {
                value.parse::<IpNet>().map_err(|error| {
                    HyperError::new(format!("invalid trusted proxy {value}: {error}"))
                })
            })
            .collect::<HyperResult<Vec<_>>>()?;

        let versions = self
            .versions
            .into_iter()
            .map(|(id, version)| {
                (
                    VersionId::new(id),
                    VersionConfig {
                        endpoint: version.endpoint,
                        health: version.health,
                    },
                )
            })
            .collect();
        let mut drain = DrainConfig::default();
        if let Some(value) = self.drain.timeout_seconds {
            drain.timeout = Duration::from_secs(value);
        }
        if let Some(value) = self.drain.force_close_streams {
            drain.force_close_streams = value;
        }
        Ok(RuntimeConfig {
            revision,
            server,
            active_version: VersionId::new(self.default_version),
            versions,
            drain,
        })
    }
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileServerConfig {
    listen: Option<SocketAddr>,
    max_connections: Option<usize>,
    header_read_timeout_ms: Option<u64>,
    version_connect_timeout_ms: Option<u64>,
    version_response_header_timeout_ms: Option<u64>,
    version_health_timeout_ms: Option<u64>,
    shutdown_timeout_seconds: Option<u64>,
    trusted_proxies: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileDrainConfig {
    timeout_seconds: Option<u64>,
    force_close_streams: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileVersionConfig {
    endpoint: String,
    health: String,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
default_version = "blue"

[server]
listen = "127.0.0.1:8088"
trusted_proxies = ["127.0.0.0/8"]

[drain]
timeout_seconds = 900
force_close_streams = true

[versions.blue]
endpoint = "http://127.0.0.1:9101"
health = "http://127.0.0.1:9101/ready"
"#;

    /// TOML 声明必须完整映射到运行时配置与明确默认值。
    #[test]
    fn toml_config_maps_runtime_contract() {
        let file: FileConfig = toml::from_str(CONFIG).expect("config should parse");
        let runtime = file
            .into_runtime(ConfigRevision { value: 7 })
            .expect("runtime should build");
        assert_eq!(runtime.revision.value, 7);
        assert_eq!(runtime.active_version.value.as_ref(), "blue");
        assert_eq!(runtime.server.listen.to_string(), "127.0.0.1:8088");
        assert_eq!(runtime.server.trusted_proxies.len(), 1);
        assert_eq!(runtime.drain.timeout.as_secs(), 900);
        assert!(runtime.drain.force_close_streams);
        assert_eq!(runtime.versions.len(), 1);
    }

    /// 非法可信代理不能在运行时被静默忽略。
    #[test]
    fn invalid_trusted_proxy_rejects_config() {
        let source = CONFIG.replace("127.0.0.0/8", "not-a-network");
        let file: FileConfig = toml::from_str(&source).expect("toml syntax should remain valid");
        assert!(file.into_runtime(ConfigRevision::INITIAL).is_err());
    }
}
