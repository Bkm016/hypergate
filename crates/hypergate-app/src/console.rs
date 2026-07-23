//! Version app 控制台接入。
//!
//! 该模块把通用 CLI 能力包装成 version app SDK。业务通常只需要使用
//! `VersionAppConsole::builder` 追加指令,不需要直接组装 `CommandRegistry`
//! 或底层命令注册表。
//!
//! @author sky

use std::sync::Arc;

use hypergate_cli::command::{
    CommandContext, CommandFuture, CommandOutput, CommandRegistry, CompletionKind,
    CompletionProvider, RegisteredCommand,
};
use hypergate_cli::console::{CommandState, ConsoleOptions, control_loop, spawn_console};
use hypergate_cli::format::render_panel;
use hypergate_cli::hypergate_commands;
use hypergate_core::{HyperError, HyperResult};

use crate::options::VersionAppOptions;

/// Version app 控制台配置。
///
/// 这是 `VersionApp` 启动控制台所需的完整配置。普通业务代码优先使用
/// `VersionAppConsole::builder` 构造它,只有需要完全替换命令注册表、
/// 状态对象或控制通道时才直接调用 `new`。
pub struct VersionAppConsole {
    /// 命令注册表。
    ///
    /// 包含 SDK 默认命令和业务追加命令。命令树也是 help 和补全的唯一
    /// 真源,不要在业务层维护第二套 help 列表。
    pub registry: CommandRegistry,
    /// 命令状态对象。
    ///
    /// 默认 builder 会注入 `VersionAppCommandState<T>`。自定义命令通过
    /// `CommandContext::state` 读取该状态。
    pub state: CommandState,
    /// 补全提供器。
    ///
    /// 用于给命令参数提供候选值。没有动态参数时可以使用默认空补全。
    pub completion_provider: Arc<dyn CompletionProvider>,
    /// 控制台运行选项。
    ///
    /// 包含 prompt 和启动 banner。
    pub options: ConsoleOptions,
}

/// Version app 指令状态。
///
/// 自定义命令通过 `CommandContext::state::<VersionAppCommandState<T>>()`
/// 读取该类型。`options` 提供 SDK 管理的进程信息,`app` 是业务
/// 传入的状态对象。
pub struct VersionAppCommandState<T> {
    /// 当前 version app 启动参数。
    pub options: VersionAppOptions,
    /// 用户注入的业务状态。
    ///
    /// 业务可以放配置快照、指标句柄、缓存句柄或只读运行状态。需要并发
    /// 修改时应自行选择合适的同步原语。
    pub app: T,
}

/// Version app 控制台构造器。
///
/// 默认会注册 SDK 自带的 `show status`、`help` 和 `exit`。业务通过
/// `command` 或 `commands` 追加自己的指令,无需复制默认命令表。业务
/// 指令推荐使用 `hypergate_command!` 或 `hypergate_commands!` 创建。
pub struct VersionAppConsoleBuilder<T = ()> {
    /// 当前 version app 启动参数。
    options: VersionAppOptions,
    /// 用户注入的业务状态。
    app: T,
    /// 当前控制台命令列表。
    commands: Vec<RegisteredCommand>,
    /// 参数补全提供器。
    completion_provider: Arc<dyn CompletionProvider>,
    /// 控制台运行选项。
    console_options: ConsoleOptions,
}

impl VersionAppConsole {
    /// 创建 version app 控制台配置。
    ///
    /// 这是最低层构造函数,调用方需要自己保证命令状态类型和 handler
    /// 中的 `CommandContext::state` 类型一致。业务二开通常不需要直接用它。
    pub fn new(
        registry: CommandRegistry,
        state: CommandState,
        completion_provider: Arc<dyn CompletionProvider>,
        options: ConsoleOptions,
    ) -> Self {
        Self {
            registry,
            state,
            completion_provider,
            options,
        }
    }

    /// 创建默认 version app 控制台配置。
    ///
    /// 该控制台只包含 SDK 默认指令,适合没有业务控制命令的 version app。
    pub fn default_for(options: &VersionAppOptions) -> Self {
        Self::builder(options).build()
    }

    /// 创建可扩展 version app 控制台构造器。
    ///
    /// builder 会自动带上 SDK 默认命令。业务追加命令后,help 和补全会
    /// 自动从同一命令树生成。
    pub fn builder(options: &VersionAppOptions) -> VersionAppConsoleBuilder {
        VersionAppConsoleBuilder::new(options.clone(), ())
    }

    /// 启动控制台和本地控制循环。
    ///
    /// 该方法由 `VersionApp::run` 调用。它会启动交互式控制台线程,
    /// 控制台输入和业务命令共用同一套命令注册表。
    pub async fn spawn(self) -> HyperResult<()> {
        let (console_tx, console_rx) = std::sync::mpsc::channel();
        let registry = Arc::new(self.registry);
        spawn_console(
            console_tx,
            self.completion_provider,
            registry.clone(),
            self.options.clone(),
        );
        tokio::spawn(control_loop(registry, self.state, console_rx));
        Ok(())
    }
}

impl<T> VersionAppConsoleBuilder<T>
where
    T: Send + Sync + 'static,
{
    /// 创建 version app 控制台构造器。
    ///
    /// 直接使用该函数时可以一次性传入业务状态。更常见的写法是
    /// `VersionAppConsole::builder(options).state(app_state)`。
    pub fn new(options: VersionAppOptions, app: T) -> Self {
        let prompt = format!("{}> ", options.name);
        let banner = format!("{} started  listen=http://{}", options.name, options.listen);
        Self {
            options,
            app,
            commands: default_commands::<T>(),
            completion_provider: Arc::new(EmptyCompletionProvider),
            console_options: ConsoleOptions::new(prompt, banner),
        }
    }

    /// 替换用户业务状态类型。
    ///
    /// 自定义命令可通过 `VersionAppCommandState<T>` 读取该状态。该方法
    /// 会保留已经追加的业务命令、补全提供器和控制台选项。
    pub fn state<N>(self, app: N) -> VersionAppConsoleBuilder<N>
    where
        N: Send + Sync + 'static,
    {
        let custom_commands = self
            .commands
            .into_iter()
            .skip(DEFAULT_VERSION_COMMAND_COUNT)
            .collect::<Vec<_>>();
        let mut next = VersionAppConsoleBuilder::new(self.options, app);
        next.commands.extend(custom_commands);
        next.completion_provider = self.completion_provider;
        next.console_options = self.console_options;
        next
    }

    /// 追加一个用户自定义命令。
    ///
    /// SDK 默认命令会保留在命令树中。命令 handler 应使用
    /// `VersionAppCommandState<T>` 读取业务状态。单条命令推荐使用
    /// `hypergate_command!` 创建。
    pub fn command(mut self, command: RegisteredCommand) -> Self {
        self.commands.push(command);
        self
    }

    /// 批量追加用户自定义命令。
    ///
    /// 适合用 `hypergate_commands!` 内联注册,也适合传入静态命令表。
    /// 示例见项目根 `src/commands.rs`。
    pub fn commands(mut self, commands: &[RegisteredCommand]) -> Self {
        self.commands.extend_from_slice(commands);
        self
    }

    /// 替换参数补全提供器。
    ///
    /// 当业务命令包含动态参数时使用。补全候选只影响交互式控制台,
    /// 不影响命令执行。
    pub fn completion_provider(mut self, provider: Arc<dyn CompletionProvider>) -> Self {
        self.completion_provider = provider;
        self
    }

    /// 替换控制台运行选项。
    ///
    /// 用于修改 prompt 或启动 banner。
    pub fn console_options(mut self, options: ConsoleOptions) -> Self {
        self.console_options = options;
        self
    }

    /// 构建 version app 控制台配置。
    ///
    /// 构建后命令注册表不可变,运行期只读。新增命令应在调用 `build`
    /// 前完成注册。
    pub fn build(self) -> VersionAppConsole {
        let state = Arc::new(VersionAppCommandState {
            options: self.options,
            app: self.app,
        });
        VersionAppConsole::new(
            CommandRegistry::from_commands(self.commands),
            state,
            self.completion_provider,
            self.console_options,
        )
    }
}

/// 不提供动态参数补全的默认实现。
struct EmptyCompletionProvider;

/// SDK 默认指令数量,用于替换业务状态类型时保留用户追加指令。
const DEFAULT_VERSION_COMMAND_COUNT: usize = 4;

impl CompletionProvider for EmptyCompletionProvider {
    /// 默认补全提供器不返回任何参数候选。
    fn complete(&self, _kind: CompletionKind, _prefix: &str) -> Vec<String> {
        Vec::new()
    }
}

/// 创建 SDK 默认 version app 命令。
fn default_commands<T>() -> Vec<RegisteredCommand>
where
    T: Send + Sync + 'static,
{
    hypergate_commands![
        show => show_help, "inspect version app";
        show status => status::<T>, "version app status";
        help => hypergate_cli::command::help, "show command tree", usage: "help [path]", arguments: Any;
        exit => hypergate_cli::command::exit, "show stop hint", scope: Console;
    ]
    .to_vec()
}

/// 将 `show` 根命令收束到自动生成的子树帮助。
fn show_help<'a>(context: CommandContext<'a>, args: &'a [&'a str]) -> CommandFuture<'a> {
    Box::pin(async move { hypergate_cli::command::scoped_help(context, args, &["show"]) })
}

/// 输出当前 version app 的 SDK 状态快照。
fn status<'a, T>(context: CommandContext<'a>, _args: &'a [&'a str]) -> CommandFuture<'a>
where
    T: Send + Sync + 'static,
{
    Box::pin(async move {
        let state = context.state::<VersionAppCommandState<T>>()?;
        Ok(CommandOutput {
            summary: format!("app={}", state.options.name),
            rendered: version_status::<T>(state)?,
        })
    })
}

/// 渲染 version app 状态面板。
fn version_status<T>(state: &(dyn std::any::Any + Send + Sync)) -> HyperResult<String>
where
    T: Send + Sync + 'static,
{
    let state = state
        .downcast_ref::<VersionAppCommandState<T>>()
        .ok_or_else(|| HyperError::new("version command state type mismatch"))?;
    Ok(render_panel(
        "Version Status",
        vec![
            ("app".to_owned(), state.options.name.to_string()),
            (
                "listen".to_owned(),
                format!("http://{}", state.options.listen),
            ),
        ],
        Vec::new(),
    ))
}
