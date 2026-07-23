//! Gateway 持久化运行状态。

use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use hypergate_core::{ConfigRevision, HyperError, HyperResult, VersionId};
use serde::{Deserialize, Serialize};

const STATE_SCHEMA_VERSION: u32 = 1;

/// Gateway 重启后需要恢复的最小运行状态。
#[derive(Clone)]
pub(crate) struct PersistedState {
    /// 当前状态修订号。
    pub(crate) revision: ConfigRevision,
    /// 当前接收新请求的版本。
    pub(crate) active_version: VersionId,
    /// 最近版本切换历史，按旧到新排列。
    pub(crate) history: Vec<VersionId>,
}

/// 原子读写 Gateway 状态文件。
pub(crate) struct StateStore {
    path: PathBuf,
}

impl StateStore {
    /// 创建状态存储。
    pub(crate) fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// 读取状态；文件不存在时以配置默认版本初始化并立即持久化。
    pub(crate) fn load_or_initialize(
        &self,
        default_version: VersionId,
    ) -> HyperResult<PersistedState> {
        match fs::read(&self.path) {
            Ok(bytes) => decode_state(&self.path, &bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let state = PersistedState {
                    revision: ConfigRevision::INITIAL,
                    active_version: default_version,
                    history: Vec::new(),
                };
                self.save(&state)?;
                Ok(state)
            }
            Err(error) => Err(HyperError::new(format!(
                "read state {} failed: {error}",
                self.path.display()
            ))),
        }
    }

    /// 通过同目录临时文件原子保存状态。
    pub(crate) fn save(&self, state: &PersistedState) -> HyperResult<()> {
        let parent = self.path.parent().unwrap_or_else(|| Path::new("."));
        let mut temporary = tempfile::NamedTempFile::new_in(parent)
            .map_err(|error| HyperError::new(format!("create state temp file failed: {error}")))?;
        restrict_permissions(temporary.as_file())?;
        let file = StateFile {
            schema_version: STATE_SCHEMA_VERSION,
            revision: state.revision.value,
            active_version: state.active_version.value.to_string(),
            history: state
                .history
                .iter()
                .map(|version| version.value.to_string())
                .collect(),
        };
        {
            let mut writer = BufWriter::new(temporary.as_file_mut());
            serde_json::to_writer_pretty(&mut writer, &file)
                .map_err(|error| HyperError::new(format!("encode state failed: {error}")))?;
            writer
                .write_all(b"\n")
                .map_err(|error| HyperError::new(format!("write state failed: {error}")))?;
            writer
                .flush()
                .map_err(|error| HyperError::new(format!("flush state failed: {error}")))?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| HyperError::new(format!("sync state failed: {error}")))?;
        temporary.persist(&self.path).map_err(|error| {
            HyperError::new(format!(
                "persist state {} failed: {}",
                self.path.display(),
                error.error
            ))
        })?;
        sync_parent(parent)?;
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StateFile {
    schema_version: u32,
    revision: u64,
    active_version: String,
    history: Vec<String>,
}

fn decode_state(path: &Path, bytes: &[u8]) -> HyperResult<PersistedState> {
    let file: StateFile = serde_json::from_slice(bytes).map_err(|error| {
        HyperError::new(format!("parse state {} failed: {error}", path.display()))
    })?;
    if file.schema_version != STATE_SCHEMA_VERSION {
        return Err(HyperError::new(format!(
            "unsupported state schema version: {}",
            file.schema_version
        )));
    }
    if file.revision == 0 || file.active_version.trim().is_empty() {
        return Err(HyperError::new(
            "state revision and active version are required",
        ));
    }
    if file.history.iter().any(|version| version.trim().is_empty()) {
        return Err(HyperError::new("state history contains an empty version"));
    }
    Ok(PersistedState {
        revision: ConfigRevision {
            value: file.revision,
        },
        active_version: VersionId::new(file.active_version),
        history: file.history.into_iter().map(VersionId::new).collect(),
    })
}

#[cfg(unix)]
fn restrict_permissions(file: &fs::File) -> HyperResult<()> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| HyperError::new(format!("set state permissions failed: {error}")))
}

#[cfg(not(unix))]
fn restrict_permissions(_file: &fs::File) -> HyperResult<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> HyperResult<()> {
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| HyperError::new(format!("sync state directory failed: {error}")))
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> HyperResult<()> {
    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    /// 首次启动写入默认版本，后续保存必须原子覆盖并完整恢复历史。
    #[test]
    fn state_round_trip_restores_active_revision_and_history() {
        let directory = tempfile::tempdir().expect("state temp directory");
        let store = StateStore::new(directory.path().join("state.json"));
        let initial = store
            .load_or_initialize(VersionId::new("blue"))
            .expect("state should initialize");
        assert_eq!(initial.active_version.value.as_ref(), "blue");
        let next = PersistedState {
            revision: ConfigRevision { value: 9 },
            active_version: VersionId::new("green"),
            history: vec![VersionId::new("blue")],
        };
        store.save(&next).expect("state should save");
        let restored = store
            .load_or_initialize(VersionId::new("ignored"))
            .expect("state should restore");
        assert_eq!(restored.revision.value, 9);
        assert_eq!(restored.active_version.value.as_ref(), "green");
        assert_eq!(restored.history[0].value.as_ref(), "blue");
    }

    /// 损坏状态必须失败关闭，不能回退到配置默认版本。
    #[test]
    fn corrupt_state_does_not_fall_back_to_default() {
        let directory = tempfile::tempdir().expect("state temp directory");
        let path = directory.path().join("state.json");
        fs::write(&path, b"not-json").expect("corrupt state should write");
        let store = StateStore::new(path);
        assert!(store.load_or_initialize(VersionId::new("blue")).is_err());
    }
}
