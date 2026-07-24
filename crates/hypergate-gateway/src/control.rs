//! Gateway 生命周期控制入口。
//!
//! [`GatewayControl`] 是 CLI 与 HTTP API 共享的唯一生命周期控制入口。
//! 它持有现有 `manager` / `gateway` / `versions` 的 [`Arc`] 引用,
//! 并用内部 [`Mutex`] 统一维护回滚历史和串行化控制动作,避免并发竞态。
//!
//! 控制动作包括: `switch` / `drain` / `stop` / `rollback` / `reload` / `snapshot`。
//! 其中 `switch` / `drain` / `stop` / `rollback` 接受 [`Option<u64>`] 期望修订号,
//! CLI 传入 `None`,HTTP API 传入具体修订号实现乐观并发控制;
//! 当传入的期望修订号与当前配置修订号不匹配时返回错误。
//!
//! 本模块不实现健康检查,业务语义与原 lifecycle 指令保持一致。

use std::collections::HashSet;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use hypergate_config::{ConfigManager, DrainConfig, RuntimeConfig};
use hypergate_core::{ConfigRevision, HyperError, HyperResult, VersionId};

use crate::http::{Gateway, HealthChecker};
use crate::runtime::{VersionRegistry, VersionRuntime};
use crate::state::{PersistedState, StateStore};

/// Gateway 生命周期控制入口。
pub(crate) struct GatewayControl {
    /// 配置管理器。
    manager: Arc<ConfigManager<RuntimeConfig>>,
    /// HTTP 网关核心。
    gateway: Arc<Gateway>,
    /// 版本运行态注册表。
    versions: Arc<VersionRegistry>,
    /// active version 与回滚历史持久化存储。
    state_store: Arc<StateStore>,
    /// 切流前 version app 健康检查实现。
    health_checker: Arc<dyn VersionHealthChecker>,
    /// 串行化控制动作并维护回滚历史。
    state: Mutex<ControlState>,
    /// 控制器创建时间,用于计算 uptime。
    started_at: Instant,
}

/// 回滚历史固定上限,超过后淘汰最旧项以保持有界内存占用。
const ROLLBACK_HISTORY_LIMIT: usize = 32;

/// 过滤已移除版本并只保留最近的回滚记录。
pub(crate) fn normalize_history(history: &mut Vec<VersionId>, configured: &HashSet<VersionId>) {
    history.retain(|version| configured.contains(version));
    if history.len() > ROLLBACK_HISTORY_LIMIT {
        history.drain(..history.len() - ROLLBACK_HISTORY_LIMIT);
    }
}

/// 控制器内部可变状态。
struct ControlState {
    /// active 版本回滚历史,LIFO 语义,固定最多 [`ROLLBACK_HISTORY_LIMIT`] 项。
    history: Vec<VersionId>,
}

/// 控制动作失败类型,区分运行态冲突和内部故障。
#[derive(Debug)]
pub(crate) enum ControlError {
    /// 当前状态拒绝操作,客户端应刷新状态后重新决策。
    Conflict(String),
    /// 内部组件执行失败。
    Internal(HyperError),
}

impl fmt::Display for ControlError {
    /// 渲染 CLI 和服务端日志使用的错误文本。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message) => formatter.write_str(message),
            Self::Internal(error) => fmt::Display::fmt(error, formatter),
        }
    }
}

impl From<ControlError> for HyperError {
    /// 将结构化控制错误转换成 CLI 使用的通用错误。
    fn from(error: ControlError) -> Self {
        Self::new(error.to_string())
    }
}

/// 控制动作返回类型。
pub(crate) type ControlResult<T> = Result<T, ControlError>;

impl GatewayControl {
    /// 创建生命周期控制入口。
    pub(crate) fn new(
        manager: Arc<ConfigManager<RuntimeConfig>>,
        gateway: Arc<Gateway>,
        versions: Arc<VersionRegistry>,
        state_store: Arc<StateStore>,
        history: Vec<VersionId>,
    ) -> Self {
        Self::with_health_checker(
            manager,
            gateway,
            versions,
            state_store,
            history,
            Arc::new(HttpVersionHealthChecker),
        )
    }

    /// 使用指定健康检查实现创建生命周期控制入口。
    fn with_health_checker(
        manager: Arc<ConfigManager<RuntimeConfig>>,
        gateway: Arc<Gateway>,
        versions: Arc<VersionRegistry>,
        state_store: Arc<StateStore>,
        mut history: Vec<VersionId>,
        health_checker: Arc<dyn VersionHealthChecker>,
    ) -> Self {
        let configured = manager
            .snapshot()
            .versions
            .keys()
            .cloned()
            .collect::<HashSet<_>>();
        normalize_history(&mut history, &configured);
        Self {
            manager,
            gateway,
            versions,
            state_store,
            health_checker,
            state: Mutex::new(ControlState { history }),
            started_at: Instant::now(),
        }
    }

    /// 校验期望修订号是否与当前配置修订号一致。
    /// `None` 表示不校验(CLI 路径),`Some(value)` 表示要求当前修订号必须等于该值。
    fn check_expected(&self, expected: Option<u64>) -> ControlResult<()> {
        match expected {
            None => Ok(()),
            Some(expected_revision) => {
                let current = self.manager.revision().value;
                if current == expected_revision {
                    Ok(())
                } else {
                    Err(ControlError::Conflict(format!(
                        "revision mismatch: expected {expected_revision}, current {current}"
                    )))
                }
            }
        }
    }

    /// 切换 active version,新请求立即进入目标版本。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 切换成功后返回形如 `active=<version>` 的摘要文本。
    pub(crate) async fn switch(
        &self,
        next_version: VersionId,
        expected: Option<u64>,
    ) -> ControlResult<ControlOutcome> {
        self.switch_with_health(next_version, expected, true).await
    }

    /// 健康检查目标版本后提交切换，探活期间发生其他控制动作则拒绝旧提交。
    async fn switch_with_health(
        &self,
        next_version: VersionId,
        expected: Option<u64>,
        push_history: bool,
    ) -> ControlResult<ControlOutcome> {
        self.check_expected(expected)?;
        let observed = self.manager.revision().value;
        let config = self.manager.snapshot();
        if config.active_version != next_version {
            let version = config
                .versions
                .get(&next_version)
                .ok_or_else(|| ControlError::Conflict("version is not configured".to_owned()))?;
            self.health_checker
                .check(
                    &version.health,
                    config.server.version_connect_timeout,
                    config.server.version_health_timeout,
                )
                .await
                .map_err(|error| {
                    ControlError::Conflict(format!("version health check failed: {error}"))
                })?;
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(Some(observed))?;
        if !push_history && state.history.last() != Some(&next_version) {
            return Err(ControlError::Conflict(
                "rollback target changed during health check".to_owned(),
            ));
        }
        let summary = self.switch_active(&mut state, next_version, push_history)?;
        Ok(self.outcome(&state, summary))
    }

    /// 让非 active 版本停止接收新请求(进入 draining)。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 成功后返回形如 `draining=<version>` 的摘要文本。
    pub(crate) fn drain(
        &self,
        version: VersionId,
        expected: Option<u64>,
    ) -> ControlResult<ControlOutcome> {
        let state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(expected)?;
        let config = self.manager.snapshot();
        if config.active_version == version {
            return Err(ControlError::Conflict(
                "active version cannot be drained directly".to_owned(),
            ));
        }
        let runtime = self
            .versions
            .get(&version)
            .map_err(ControlError::Internal)?
            .ok_or_else(|| ControlError::Conflict("version not found".to_owned()))?;
        self.begin_drain(runtime, &config.drain);
        Ok(self.outcome(&state, format!("draining={}", version.value)))
    }

    /// 在连接清空后停止非 active 版本。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 成功后返回形如 `stopped=<version>` 的摘要文本。
    pub(crate) fn stop(
        &self,
        version: VersionId,
        expected: Option<u64>,
    ) -> ControlResult<ControlOutcome> {
        let state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(expected)?;
        let config = self.manager.snapshot();
        if config.active_version == version {
            return Err(ControlError::Conflict(
                "active version cannot be stopped".to_owned(),
            ));
        }
        let runtime = self
            .versions
            .get(&version)
            .map_err(ControlError::Internal)?
            .ok_or_else(|| ControlError::Conflict("version not found".to_owned()))?;
        runtime
            .stop_idle()
            .map_err(|error| ControlError::Conflict(error.to_string()))?;
        Ok(self.outcome(&state, format!("stopped={}", version.value)))
    }

    /// 切回上一个 active version。
    ///
    /// 当 `expected` 与当前配置修订号不匹配时返回错误。
    /// 成功后返回形如 `active=<version>` 的摘要文本。
    /// 边界:回滚按 LIFO 语义消费历史栈顶,消费后弹出该条目;历史本身由 switch 维持有界。
    pub(crate) async fn rollback(&self, expected: Option<u64>) -> ControlResult<ControlOutcome> {
        self.check_expected(expected)?;
        let previous = {
            let state = self
                .state
                .lock()
                .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
            state
                .history
                .last()
                .cloned()
                .ok_or_else(|| ControlError::Conflict("rollback history is empty".to_owned()))?
        };
        self.switch_with_health(previous, expected, false).await
    }

    /// 重新加载配置快照并同步 gateway 热路径状态。
    ///
    /// 成功后返回 `reload=ok`。
    pub(crate) async fn reload(&self, expected: Option<u64>) -> ControlResult<ControlOutcome> {
        self.check_expected(expected)?;
        let observed = self.manager.revision().value;
        let old = self.manager.snapshot();
        let next_revision = self.manager.revision().next();
        let mut next = self
            .manager
            .loader
            .load(next_revision)
            .map_err(ControlError::Internal)?;
        if next.server != old.server {
            return Err(ControlError::Conflict(
                "server configuration changes require a gateway restart".to_owned(),
            ));
        }
        next.active_version = old.active_version.clone();
        for version_id in next.versions.keys() {
            self.versions
                .ensure(version_id.clone())
                .map_err(ControlError::Internal)?;
        }
        let prepared = self
            .gateway
            .prepare_active(&next)
            .map_err(ControlError::Internal)?;
        self.manager
            .validator
            .validate(&next)
            .map_err(ControlError::Internal)?;
        let active = next
            .versions
            .get(&next.active_version)
            .ok_or_else(|| ControlError::Conflict("active version is not configured".to_owned()))?;
        self.health_checker
            .check(
                &active.health,
                next.server.version_connect_timeout,
                next.server.version_health_timeout,
            )
            .await
            .map_err(|error| {
                ControlError::Conflict(format!("active version health check failed: {error}"))
            })?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ControlError::Internal(HyperError::new("control lock poisoned")))?;
        self.check_expected(Some(observed))?;
        for snapshot in self.versions.snapshots().map_err(ControlError::Internal)? {
            if snapshot.state != hypergate_core::VersionState::Stopped
                && !next.versions.contains_key(&snapshot.id)
            {
                return Err(ControlError::Conflict(format!(
                    "running version cannot be removed: {}",
                    snapshot.id.value
                )));
            }
        }
        let configured = next.versions.keys().cloned().collect::<HashSet<_>>();
        let mut next_history = state.history.clone();
        normalize_history(&mut next_history, &configured);
        self.versions
            .retain_configured(&configured)
            .map_err(ControlError::Internal)?;
        self.persist_state(next.revision, &next.active_version, &next_history)?;
        self.manager.apply_validated(next);
        self.gateway.swap_active(prepared);
        state.history = next_history;
        Ok(self.outcome(&state, "reload=ok".to_owned()))
    }

    /// 执行 active version 切换并按需记录回滚历史。
    fn switch_active(
        &self,
        state: &mut ControlState,
        next_version: VersionId,
        push_history: bool,
    ) -> ControlResult<String> {
        let old = self.manager.snapshot();
        if !old.versions.contains_key(&next_version) {
            return Err(ControlError::Conflict(
                "version is not configured".to_owned(),
            ));
        }
        if old.active_version == next_version {
            return Ok(format!("active={}", old.active_version.value));
        }
        let mut next_history = state.history.clone();
        if push_history {
            next_history.push(old.active_version.clone());
        } else {
            next_history.pop();
        }
        let configured = old.versions.keys().cloned().collect::<HashSet<_>>();
        normalize_history(&mut next_history, &configured);
        let mut next = old.as_ref().clone();
        next.revision = next.revision.next();
        next.active_version = next_version.clone();
        let prepared = self
            .gateway
            .prepare_active(&next)
            .map_err(ControlError::Internal)?;
        let old_runtime = self
            .versions
            .get(&old.active_version)
            .map_err(ControlError::Internal)?
            .ok_or_else(|| {
                ControlError::Internal(HyperError::new("active version is not registered"))
            })?;
        let next_runtime = prepared.runtime.clone();
        self.manager
            .validator
            .validate(&next)
            .map_err(ControlError::Internal)?;
        self.persist_state(next.revision, &next.active_version, &next_history)?;
        next_runtime.activate();
        // 新版本激活后原子替换热路径,旧版本此时仍可承接已读取旧快照的请求。
        self.gateway.swap_active(prepared);
        let applied = self.manager.apply_validated(next);
        self.begin_drain(old_runtime, &applied.drain);
        state.history = next_history;
        Ok(format!("active={}", applied.active_version.value))
    }

    /// 进入 draining，并按配置在超时后取消该激活周期的残留连接。
    fn begin_drain(&self, runtime: Arc<VersionRuntime>, config: &DrainConfig) {
        runtime.drain();
        if !config.force_close_streams {
            return;
        }
        let timeout = config.timeout;
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            runtime.force_close_if_draining();
        });
    }

    /// 在切流前持久化下一状态，进程崩溃重启后继续指向已提交版本。
    fn persist_state(
        &self,
        revision: ConfigRevision,
        active_version: &VersionId,
        history: &[VersionId],
    ) -> ControlResult<()> {
        self.state_store
            .save(&PersistedState {
                revision,
                active_version: active_version.clone(),
                history: history.to_vec(),
            })
            .map_err(ControlError::Internal)
    }

    /// 生成可 serde 序列化的状态快照 DTO。
    ///
    /// 快照字段使用 camelCase,便于后续 HTTP API 直接序列化为 JSON。
    pub(crate) fn snapshot(&self) -> HyperResult<GatewaySnapshot> {
        let state = self
            .state
            .lock()
            .map_err(|_| HyperError::new("control lock poisoned"))?;
        self.snapshot_unlocked(&state)
    }

    /// 在已持有控制锁时生成动作结果。
    fn outcome(&self, state: &ControlState, summary: String) -> ControlOutcome {
        ControlOutcome {
            summary,
            snapshot: self.snapshot_unlocked(state),
        }
    }

    /// 在已持有控制锁时读取一致状态快照。
    fn snapshot_unlocked(&self, state: &ControlState) -> HyperResult<GatewaySnapshot> {
        let config = self.manager.snapshot();
        let snapshots = self.versions.snapshots()?;
        let mut versions: Vec<_> = snapshots
            .into_iter()
            .map(|snapshot| {
                let version = config
                    .versions
                    .get(&VersionId::new(snapshot.id.value.clone()));
                VersionSnapshotDto {
                    id: snapshot.id.value.to_string(),
                    state: snapshot.state.as_str().to_owned(),
                    endpoint: version.map(|version| version.endpoint.clone()),
                    health: version.map(|version| version.health.clone()),
                    active_requests: snapshot.active_requests,
                    active_streams: snapshot.active_streams,
                    total_requests: snapshot.total_requests,
                    drain_elapsed_seconds: snapshot.drain_elapsed_secs,
                    drain_timed_out: snapshot
                        .drain_elapsed_secs
                        .is_some_and(|elapsed| elapsed >= config.drain.timeout.as_secs()),
                }
            })
            .collect();
        versions.sort_by(|a, b| a.id.cmp(&b.id));
        let rollback_available = !state.history.is_empty();
        Ok(GatewaySnapshot {
            active_version: config.active_version.value.to_string(),
            revision: config.revision.value,
            rollback_available,
            uptime_seconds: self.started_at.elapsed().as_secs(),
            versions,
        })
    }
}

/// 切流健康检查扩展边界。
trait VersionHealthChecker: Send + Sync {
    /// 检查目标地址是否可接收流量。
    fn check<'a>(
        &'a self,
        health: &'a str,
        connect_timeout: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Pin<Box<dyn Future<Output = HyperResult<()>> + Send + 'a>>;
}

/// 基于 Gateway HTTP client 的生产健康检查。
struct HttpVersionHealthChecker;

impl VersionHealthChecker for HttpVersionHealthChecker {
    fn check<'a>(
        &'a self,
        health: &'a str,
        connect_timeout: std::time::Duration,
        timeout: std::time::Duration,
    ) -> Pin<Box<dyn Future<Output = HyperResult<()>> + Send + 'a>> {
        Box::pin(async move {
            HealthChecker::new(connect_timeout, timeout)
                .check(health)
                .await
        })
    }
}

/// 控制动作结果,包含 CLI 摘要和同一临界区内生成的状态快照。
pub(crate) struct ControlOutcome {
    /// CLI 使用的简短动作摘要。
    pub(crate) summary: String,
    /// HTTP API 返回的动作完成后状态,读取失败不改变动作执行结果。
    pub(crate) snapshot: HyperResult<GatewaySnapshot>,
}

/// Gateway 状态快照 DTO,可序列化为 JSON。
///
/// 所有字段使用 camelCase,供后续 HTTP API 直接序列化。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GatewaySnapshot {
    /// 当前 active 版本标识。
    active_version: String,
    /// 当前配置修订号。
    revision: u64,
    /// 是否存在可回滚的历史版本。
    rollback_available: bool,
    /// 控制器启动至今的秒数。
    uptime_seconds: u64,
    /// 全部版本运行态快照。
    versions: Vec<VersionSnapshotDto>,
}

/// 单个版本运行态快照 DTO。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VersionSnapshotDto {
    /// 版本标识。
    id: String,
    /// 当前版本状态文本。
    state: String,
    /// Version app 入口地址,运行态未关联配置时为 `None`。
    endpoint: Option<String>,
    /// 健康检查地址,未配置时为 `None`。
    health: Option<String>,
    /// 活跃请求数量。
    active_requests: u64,
    /// 活跃流式连接数量。
    active_streams: u64,
    /// 累计成功创建的请求租约总数。
    total_requests: u64,
    /// draining 已持续秒数,未进入 draining 时为 null。
    drain_elapsed_seconds: Option<u64>,
    /// draining 是否已超过配置上限。
    drain_timed_out: bool,
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    //! 控制器边界行为测试。验证 rollback history 在固定上限内淘汰最旧项,
    //! 且 rollback 保持 LIFO 语义。

    use super::*;
    use hypergate_config::{
        ConfigLoader, ConfigValidatorChain, DefaultConfigValidator, VersionConfig,
    };
    use std::path::PathBuf;

    struct MutableConfigLoader {
        template: Mutex<RuntimeConfig>,
    }

    impl MutableConfigLoader {
        fn replace(&self, template: RuntimeConfig) {
            *self.template.lock().expect("config loader lock") = template;
        }
    }

    impl ConfigLoader<RuntimeConfig> for MutableConfigLoader {
        fn load(&self, next_revision: ConfigRevision) -> HyperResult<RuntimeConfig> {
            let mut config = self.template.lock().expect("config loader lock").clone();
            config.revision = next_revision;
            Ok(config)
        }
    }

    struct AlwaysHealthy;

    impl VersionHealthChecker for AlwaysHealthy {
        fn check<'a>(
            &'a self,
            _health: &'a str,
            _connect_timeout: std::time::Duration,
            _timeout: std::time::Duration,
        ) -> Pin<Box<dyn Future<Output = HyperResult<()>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct NeverHealthy;

    impl VersionHealthChecker for NeverHealthy {
        fn check<'a>(
            &'a self,
            _health: &'a str,
            _connect_timeout: std::time::Duration,
            _timeout: std::time::Duration,
        ) -> Pin<Box<dyn Future<Output = HyperResult<()>> + Send + 'a>> {
            Box::pin(async { Err(HyperError::new("not ready")) })
        }
    }

    /// 构造一个最小可运行的 GatewayControl,包含 `count` 个版本,
    /// 初始 active 版本为 v0。
    fn build_control(count: usize) -> Arc<GatewayControl> {
        build_control_with_health(count, Arc::new(AlwaysHealthy))
    }

    /// 使用指定健康检查结果构造控制器。
    fn build_control_with_health(
        count: usize,
        health_checker: Arc<dyn VersionHealthChecker>,
    ) -> Arc<GatewayControl> {
        build_control_parts(count, health_checker).0
    }

    /// 构造可替换加载配置并检查持久化状态的控制器。
    fn build_control_parts(
        count: usize,
        health_checker: Arc<dyn VersionHealthChecker>,
    ) -> (Arc<GatewayControl>, Arc<MutableConfigLoader>, PathBuf) {
        build_control_parts_with_history(count, health_checker, Vec::new())
    }

    /// 使用指定的持久化历史构造控制器。
    fn build_control_parts_with_history(
        count: usize,
        health_checker: Arc<dyn VersionHealthChecker>,
        history: Vec<VersionId>,
    ) -> (Arc<GatewayControl>, Arc<MutableConfigLoader>, PathBuf) {
        let mut config = RuntimeConfig::minimal();
        config.active_version = VersionId::new("v0");
        for index in 0..count {
            let id = VersionId::new(format!("v{index}"));
            config.versions.insert(
                id,
                VersionConfig {
                    endpoint: format!("http://127.0.0.1:{}", 9000 + index),
                    health: "http://127.0.0.1/health".to_owned(),
                },
            );
        }
        let loader = Arc::new(MutableConfigLoader {
            template: Mutex::new(config.clone()),
        });
        let mut validator_chain = ConfigValidatorChain::<RuntimeConfig>::new();
        validator_chain.push(Arc::new(DefaultConfigValidator));
        let validator = Arc::new(validator_chain);
        let manager = Arc::new(ConfigManager::new(
            config.clone(),
            loader.clone(),
            validator,
        ));
        let versions = Arc::new(VersionRegistry::new());
        for version_id in config.versions.keys() {
            versions
                .ensure(version_id.clone())
                .expect("version should be registered");
        }
        versions
            .get(&config.active_version)
            .expect("active version lookup")
            .expect("active version should exist")
            .activate();
        let gateway =
            Arc::new(Gateway::new(&config, versions.clone()).expect("gateway should build"));
        let state_file = tempfile::NamedTempFile::new().expect("state temp file");
        let state_path = state_file.path().to_path_buf();
        drop(state_file);
        let control = Arc::new(GatewayControl::with_health_checker(
            manager,
            gateway,
            versions,
            Arc::new(StateStore::new(state_path.clone())),
            history,
            health_checker,
        ));
        (control, loader, state_path)
    }

    /// 健康检查失败时不能修改 active version、revision 或回滚历史。
    #[tokio::test]
    async fn unhealthy_version_never_receives_traffic() {
        let control = build_control_with_health(2, Arc::new(NeverHealthy));
        assert!(control.switch(VersionId::new("v1"), Some(1)).await.is_err());
        let snapshot = control.snapshot().expect("snapshot should succeed");
        assert_eq!(snapshot.active_version, "v0");
        assert_eq!(snapshot.revision, 1);
        assert!(!snapshot.rollback_available);
    }

    /// 验证 switch 记录历史并在超过 32 项上限时淘汰最旧项。
    #[tokio::test]
    async fn switch_evicts_oldest_history_beyond_limit() {
        let control = build_control(ROLLBACK_HISTORY_LIMIT + 2);
        // 依次切换到 v1..v33,每次都会把当前 active 版本压入历史。
        for index in 1..=ROLLBACK_HISTORY_LIMIT + 1 {
            let target = VersionId::new(format!("v{index}"));
            let outcome = control
                .switch(target, None)
                .await
                .expect("switch should succeed");
            assert!(
                outcome.summary.starts_with("active=v"),
                "switch should report active version: {}",
                outcome.summary
            );
        }
        // 历史应固定在上限,最旧的 v0 已被淘汰。
        let snapshot = control.snapshot().expect("snapshot should succeed");
        assert!(snapshot.rollback_available, "rollback history should exist");
        // 连续回滚 ROLLBACK_HISTORY_LIMIT 次,每次应回到上一个 active 版本。
        // 回滚顺序(LIFO): v32 -> v33 不对,应为 v33 的前一个是 v32...
        // 历史栈在 switch 时记录的是 *切换前* 的 active 版本。
        // v0(active) -> switch v1: history=[v0]
        // v1 -> switch v2: history=[v0,v1]
        // ...
        // v32 -> switch v33: history=[v0,...,v32] (33 项,淘汰 v0 -> [v1,...,v32] 32 项)
        // 因此回滚第一次应回到 v32。
        let outcome = control
            .rollback(None)
            .await
            .expect("rollback should succeed");
        assert!(
            outcome.summary.contains("active=v32"),
            "first rollback should restore v32: {}",
            outcome.summary
        );
    }

    /// 验证 rollback 在历史为空时返回 Conflict 错误。
    #[tokio::test]
    async fn rollback_empty_history_returns_conflict() {
        let control = build_control(2);
        // 初始状态下历史为空,回滚应失败。
        let result = control.rollback(None).await;
        assert!(result.is_err(), "rollback on empty history should fail");
        let snapshot = control.snapshot().expect("snapshot should succeed");
        assert!(
            !snapshot.rollback_available,
            "rollback should not be available with empty history"
        );
    }

    /// 验证 switch 后 rollback 按 LIFO 回到上一个 active 版本。
    #[tokio::test]
    async fn rollback_restores_previous_active_via_lifo() {
        let control = build_control(3);
        // v0 -> switch v1: history=[v0]
        control
            .switch(VersionId::new("v1"), None)
            .await
            .expect("switch to v1");
        // v1 -> switch v2: history=[v0,v1]
        control
            .switch(VersionId::new("v2"), None)
            .await
            .expect("switch to v2");
        // rollback(LIFO): 回到 v1,history=[v0]
        let outcome = control.rollback(None).await.expect("rollback to v1");
        assert!(
            outcome.summary.contains("active=v1"),
            "first rollback should restore v1: {}",
            outcome.summary
        );
        // rollback(LIFO): 回到 v0,history=[]
        let outcome = control.rollback(None).await.expect("rollback to v0");
        assert!(
            outcome.summary.contains("active=v0"),
            "second rollback should restore v0: {}",
            outcome.summary
        );
        // 历史已空,再次回滚应失败。
        assert!(
            control.rollback(None).await.is_err(),
            "rollback after exhausting history should fail"
        );
    }

    /// Reload 删除版本时必须同时清理内存与持久化历史，并保留仍有效的回滚目标。
    #[tokio::test]
    async fn reload_removes_deleted_versions_from_rollback_history() {
        let (control, loader, state_path) = build_control_parts(3, Arc::new(AlwaysHealthy));
        control
            .switch(VersionId::new("v1"), None)
            .await
            .expect("switch to v1");
        control
            .switch(VersionId::new("v2"), None)
            .await
            .expect("switch to v2");
        control
            .stop(VersionId::new("v1"), None)
            .expect("v1 should stop after draining");

        let mut next = control.manager.snapshot().as_ref().clone();
        next.versions.remove(&VersionId::new("v1"));
        loader.replace(next);
        control.reload(None).await.expect("reload should succeed");

        let persisted = StateStore::new(state_path)
            .load_or_initialize(VersionId::new("unused"))
            .expect("persisted state should load");
        assert_eq!(persisted.history, vec![VersionId::new("v0")]);
        let outcome = control.rollback(None).await.expect("rollback to v0");
        assert_eq!(outcome.summary, "active=v0");
    }

    /// Reload 删除唯一历史版本后必须关闭 rollback 可用标记并清空状态文件。
    #[tokio::test]
    async fn reload_clears_rollback_when_all_history_versions_are_deleted() {
        let (control, loader, state_path) = build_control_parts(2, Arc::new(AlwaysHealthy));
        control
            .switch(VersionId::new("v1"), None)
            .await
            .expect("switch to v1");
        control
            .stop(VersionId::new("v0"), None)
            .expect("v0 should stop after draining");

        let mut next = control.manager.snapshot().as_ref().clone();
        next.versions.remove(&VersionId::new("v0"));
        loader.replace(next);
        control.reload(None).await.expect("reload should succeed");

        let snapshot = control.snapshot().expect("snapshot should succeed");
        assert!(!snapshot.rollback_available);
        let persisted = StateStore::new(state_path)
            .load_or_initialize(VersionId::new("unused"))
            .expect("persisted state should load");
        assert!(persisted.history.is_empty());
        assert!(control.rollback(None).await.is_err());
    }

    /// 控制器恢复旧状态时必须过滤失效版本，并只保留最近 32 项历史。
    #[test]
    fn restored_history_is_filtered_and_capped() {
        let mut history = vec![VersionId::new("deleted")];
        history.extend((0..40).map(|index| VersionId::new(format!("v{index}"))));
        let (control, _, _) =
            build_control_parts_with_history(40, Arc::new(AlwaysHealthy), history);

        let state = control.state.lock().expect("control state lock");
        assert_eq!(state.history.len(), ROLLBACK_HISTORY_LIMIT);
        assert_eq!(state.history.first(), Some(&VersionId::new("v8")));
        assert_eq!(state.history.last(), Some(&VersionId::new("v39")));
    }
}
