//! 多版本生命周期控制指令。

use hypergate_core::{HyperError, HyperResult, VersionId};

use hypergate_cli::command::{CommandContext, CommandOutput};

use super::{gateway_state, single_arg, status_after_control};

/// 切换 active version,新请求立即进入目标版本。
pub(super) fn switch(context: CommandContext<'_>, args: &[&str]) -> HyperResult<CommandOutput> {
    let version = single_arg(args, "missing version")?;
    let summary = switch_active(context, VersionId::new(version), true)?;
    status_after_control(context, summary)
}

/// 重新加载配置快照并同步 gateway 热路径状态。
pub(super) fn reload(context: CommandContext<'_>, _args: &[&str]) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let old = state.manager.snapshot();
    let mut next = old.as_ref().clone();
    next.revision = next.revision.next();
    let applied = state.manager.apply(next)?;
    state.gateway.sync_active(applied.as_ref())?;
    status_after_control(context, "reload=ok".to_owned())
}

/// 让非 active 版本停止接收新请求。
pub(super) fn drain(context: CommandContext<'_>, args: &[&str]) -> HyperResult<CommandOutput> {
    let version = VersionId::new(single_arg(args, "missing version")?);
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    if config.active_version == version {
        return Err(HyperError::new("active version cannot be drained directly"));
    }
    state.runtime.drain_version(version.clone())?;
    status_after_control(context, format!("draining={}", version.value))
}

/// 在连接清空后停止非 active 版本。
pub(super) fn stop(context: CommandContext<'_>, args: &[&str]) -> HyperResult<CommandOutput> {
    let version = VersionId::new(single_arg(args, "missing version")?);
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    if config.active_version == version {
        return Err(HyperError::new("active version cannot be stopped"));
    }
    state.runtime.stop_idle_version(version.clone())?;
    status_after_control(context, format!("stopped={}", version.value))
}

/// 切回上一个 active version。
pub(super) fn rollback(context: CommandContext<'_>, _args: &[&str]) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let previous = {
        let mut guard = state
            .history
            .lock()
            .map_err(|_| HyperError::new("history lock poisoned"))?;
        guard
            .pop()
            .ok_or_else(|| HyperError::new("rollback history is empty"))?
    };
    let summary = switch_active(context, previous, false)?;
    status_after_control(context, summary)
}

/// 执行 active version 切换并按需记录回滚历史。
fn switch_active(
    context: CommandContext<'_>,
    next_version: VersionId,
    push_history: bool,
) -> HyperResult<String> {
    let state = gateway_state(context)?;
    let old = state.manager.snapshot();
    if !old.versions.contains_key(&next_version) {
        return Err(HyperError::new("version is not configured"));
    }
    if old.active_version == next_version {
        return Ok(format!("active={}", old.active_version.value));
    }
    // 先切运行态,再提交配置快照,避免配置指向尚未激活的版本。
    state.runtime.switch_to(&old, next_version.clone())?;

    if push_history {
        let mut guard = state
            .history
            .lock()
            .map_err(|_| HyperError::new("history lock poisoned"))?;
        guard.push(old.active_version.clone());
    }

    let mut next = old.as_ref().clone();
    next.revision = next.revision.next();
    next.active_version = next_version;
    let applied = state.manager.apply(next)?;
    state.gateway.sync_active(applied.as_ref())?;
    Ok(format!("active={}", applied.active_version.value))
}
