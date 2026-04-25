//! 多版本注册表和请求租约分配。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use hypergate_core::{HyperError, HyperResult, VersionId};

use super::{VersionRuntime, VersionSnapshot};

/// 多版本注册表。
pub(crate) struct VersionRegistry {
    /// 已知版本运行态。
    pub(crate) versions: RwLock<HashMap<VersionId, Arc<VersionRuntime>>>,
}

impl VersionRegistry {
    /// 创建空注册表。
    pub(crate) fn new() -> Self {
        Self {
            versions: RwLock::new(HashMap::new()),
        }
    }

    /// 注册或返回已有版本。
    pub(crate) fn ensure(&self, id: VersionId) -> HyperResult<Arc<VersionRuntime>> {
        let mut versions = self
            .versions
            .write()
            .map_err(|_| HyperError::new("version registry lock poisoned"))?;
        let runtime = versions
            .entry(id.clone())
            .or_insert_with(|| Arc::new(VersionRuntime::new(id)))
            .clone();
        Ok(runtime)
    }

    /// 获取版本运行态。
    pub(crate) fn get(&self, id: &VersionId) -> HyperResult<Option<Arc<VersionRuntime>>> {
        let versions = self
            .versions
            .read()
            .map_err(|_| HyperError::new("version registry lock poisoned"))?;
        Ok(versions.get(id).cloned())
    }

    /// 获取所有版本快照。
    pub(crate) fn snapshots(&self) -> HyperResult<Vec<VersionSnapshot>> {
        let versions = self
            .versions
            .read()
            .map_err(|_| HyperError::new("version registry lock poisoned"))?;
        let mut snapshots = Vec::with_capacity(versions.len());
        for version in versions.values() {
            snapshots.push(version.snapshot()?);
        }
        snapshots.sort_by(|a, b| a.id.value.cmp(&b.id.value));
        Ok(snapshots)
    }
}

impl Default for VersionRegistry {
    /// 默认创建空版本注册表。
    fn default() -> Self {
        Self::new()
    }
}
