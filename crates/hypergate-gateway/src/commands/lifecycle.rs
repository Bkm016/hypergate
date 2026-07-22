//! 多版本生命周期控制指令。
//!
//! 所有控制动作通过 [`GatewayControl`] 执行,这里只负责解析 CLI 参数、
//! 调用 control 对应方法并把摘要文本交给状态视图刷新。

use hypergate_core::{HyperResult, VersionId};

use hypergate_cli::command::{CommandContext, CommandOutput};

use super::{gateway_state, single_arg, status_after_control};

/// 切换 active version,新请求立即进入目标版本。
pub(super) fn switch(context: CommandContext<'_>, args: &[&str]) -> HyperResult<CommandOutput> {
    let version = single_arg(args, "missing version")?;
    let state = gateway_state(context)?;
    let outcome = state.control.switch(VersionId::new(version), None)?;
    status_after_control(context, outcome.summary)
}

/// 重新加载配置快照并同步 gateway 热路径状态。
pub(super) fn reload(context: CommandContext<'_>, _args: &[&str]) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let outcome = state.control.reload()?;
    status_after_control(context, outcome.summary)
}

/// 让非 active 版本停止接收新请求。
pub(super) fn drain(context: CommandContext<'_>, args: &[&str]) -> HyperResult<CommandOutput> {
    let version = VersionId::new(single_arg(args, "missing version")?);
    let state = gateway_state(context)?;
    let outcome = state.control.drain(version, None)?;
    status_after_control(context, outcome.summary)
}

/// 在连接清空后停止非 active 版本。
pub(super) fn stop(context: CommandContext<'_>, args: &[&str]) -> HyperResult<CommandOutput> {
    let version = VersionId::new(single_arg(args, "missing version")?);
    let state = gateway_state(context)?;
    let outcome = state.control.stop(version, None)?;
    status_after_control(context, outcome.summary)
}

/// 切回上一个 active version。
pub(super) fn rollback(context: CommandContext<'_>, _args: &[&str]) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let outcome = state.control.rollback(None)?;
    status_after_control(context, outcome.summary)
}
