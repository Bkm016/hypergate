//! Gateway 默认指令注册。

use hypergate_cli::command::{
    CommandContext, CommandFuture, CommandOutput, CompletionKind, CompletionProvider,
    RegisteredCommand,
};
use hypergate_cli::hypergate_commands;
use hypergate_config::{ConfigManager, RuntimeConfig};
use hypergate_core::{HyperError, HyperResult};
use std::sync::Arc;

use crate::control::GatewayControl;
use crate::runtime::VersionRegistry;
use crate::views::format_status;

mod lifecycle;
mod view;

use lifecycle::{drain, reload, rollback, stop, switch};
use view::{config_check, config_show, status, versions};

/// Gateway 指令状态。
pub(crate) struct GatewayCommandState {
    /// 配置管理器。
    pub(crate) manager: Arc<ConfigManager<RuntimeConfig>>,
    /// 版本运行态注册表。
    pub(crate) versions: Arc<VersionRegistry>,
    /// 生命周期控制入口。
    pub(crate) control: Arc<GatewayControl>,
}

/// Gateway 指令补全提供器。
pub(crate) struct GatewayCompletionProvider {
    /// 配置管理器。
    pub(crate) manager: Arc<ConfigManager<RuntimeConfig>>,
}

impl CompletionProvider for GatewayCompletionProvider {
    /// 只为 version 参数提供补全候选。
    fn complete(&self, kind: CompletionKind, prefix: &str) -> Vec<String> {
        if kind != CompletionKind::Version {
            return Vec::new();
        }
        let config = self.manager.snapshot();
        config
            .versions
            .keys()
            .filter(|version| version.value.starts_with(prefix))
            .map(|version| version.value.to_string())
            .collect()
    }
}

/// Gateway 指令注册表。
pub(crate) const COMMANDS: &[RegisteredCommand] = hypergate_commands![
    show => show_help, "inspect runtime";
    show status => status, "gateway status";
    show versions => versions, "version endpoints and live requests";
    config => config_show, "show config";
    config check => config_check, "validate config";
    config reload => reload, "reload config";
    version => version_help, "version lifecycle";
    version switch => switch, "move new traffic", usage: "version switch <version>", completion: Version, arguments: Any;
    version drain => drain, "stop accepting new traffic", usage: "version drain <version>", completion: Version, arguments: Any;
    version stop => stop, "stop an idle version", usage: "version stop <version>", completion: Version, arguments: Any;
    version rollback => rollback, "switch back to previous active";
    help => hypergate_cli::command::help, "show command tree", usage: "help [path]", arguments: Any;
    exit => hypergate_cli::command::exit, "show stop hint", scope: Console;
];

/// 将 `show` 根命令收束到自动生成的子树帮助。
fn show_help<'a>(context: CommandContext<'a>, args: &'a [&'a str]) -> CommandFuture<'a> {
    Box::pin(async move { hypergate_cli::command::scoped_help(context, args, &["show"]) })
}

/// 将 `version` 根命令收束到自动生成的子树帮助。
fn version_help<'a>(context: CommandContext<'a>, args: &'a [&'a str]) -> CommandFuture<'a> {
    Box::pin(async move { hypergate_cli::command::scoped_help(context, args, &["version"]) })
}

/// 控制类命令执行成功后刷新状态视图。
fn status_after_control(
    context: CommandContext<'_>,
    summary: String,
) -> HyperResult<CommandOutput> {
    let state = gateway_state(context)?;
    let config = state.manager.snapshot();
    Ok(CommandOutput {
        rendered: format_status(config.as_ref(), state.versions.as_ref())?,
        summary,
    })
}

/// 从通用命令上下文中取回 gateway 专用状态。
fn gateway_state(context: CommandContext<'_>) -> HyperResult<&GatewayCommandState> {
    context.state::<GatewayCommandState>()
}

/// 读取唯一参数,并把缺失和多余参数统一转成命令错误。
fn single_arg<'a>(args: &'a [&'a str], missing: &str) -> HyperResult<&'a str> {
    if args.len() > 1 {
        return Err(HyperError::new("too many arguments"));
    }
    args.first()
        .copied()
        .ok_or_else(|| HyperError::new(missing))
}
