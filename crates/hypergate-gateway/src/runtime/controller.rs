//! 多版本生命周期协调器。

use std::sync::Arc;

use hypergate_config::RuntimeConfig;
use hypergate_core::{HyperError, HyperResult, VersionId};

use super::VersionRegistry;

/// 版本切换协调器。
pub(crate) struct RuntimeController {
    /// 版本注册表。
    pub(crate) registry: Arc<VersionRegistry>,
}

impl RuntimeController {
    /// 创建版本切换协调器。
    pub(crate) fn new(registry: Arc<VersionRegistry>) -> Self {
        Self { registry }
    }

    /// 将新版本切为 active,旧 active 版本进入 draining。
    pub(crate) fn switch_to(
        &self,
        old_config: &RuntimeConfig,
        next_version: VersionId,
    ) -> HyperResult<()> {
        if old_config.active_version == next_version {
            return Ok(());
        }
        let old = self.registry.ensure(old_config.active_version.clone())?;
        let next = self.registry.ensure(next_version)?;
        next.activate()?;
        old.drain()?;
        Ok(())
    }

    /// 手动让指定版本进入 draining。
    pub(crate) fn drain_version(&self, version: VersionId) -> HyperResult<()> {
        let version = self
            .registry
            .get(&version)?
            .ok_or_else(|| HyperError::new("version not found"))?;
        version.drain()
    }

    /// 无活跃连接时停止指定版本。
    pub(crate) fn stop_idle_version(&self, version: VersionId) -> HyperResult<()> {
        let version = self
            .registry
            .get(&version)?
            .ok_or_else(|| HyperError::new("version not found"))?;
        version.stop_idle()
    }
}
