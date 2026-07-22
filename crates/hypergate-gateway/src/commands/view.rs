//! 只读查看类指令。

use hypergate_core::HyperResult;

use hypergate_cli::command::{CommandContext, CommandOutput};
use hypergate_cli::format::render_tables;

use crate::views::{format_check_result, format_config, format_status, versions_table};

use super::gateway_state;

/// 输出 gateway 状态总览。
pub(super) fn status(context: CommandContext<'_>, _args: &[&str]) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    Ok(CommandOutput {
        summary: "console=status".to_owned(),
        rendered: format_status(config.as_ref(), state.versions.as_ref())?,
    })
}

/// 输出版本 endpoint、状态和连接计数。
pub(super) fn versions(context: CommandContext<'_>, _args: &[&str]) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    Ok(CommandOutput {
        summary: "console=versions".to_owned(),
        rendered: render_tables(vec![versions_table(
            config.as_ref(),
            state.versions.as_ref(),
        )?]),
    })
}

/// 输出当前运行配置。
pub(super) fn config_show(
    context: CommandContext<'_>,
    _args: &[&str],
) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    Ok(CommandOutput {
        summary: "console=config".to_owned(),
        rendered: format_config(config.as_ref()),
    })
}

/// 校验当前运行配置。
pub(super) fn config_check(
    context: CommandContext<'_>,
    _args: &[&str],
) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    Ok(CommandOutput {
        summary: "config=ok".to_owned(),
        rendered: format_check_result(config.as_ref())?,
    })
}
