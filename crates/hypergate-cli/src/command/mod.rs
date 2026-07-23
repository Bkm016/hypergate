//! 可复用命令注册、解析、补全和帮助渲染。

use std::any::Any;
use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use hypergate_core::{HyperError, HyperResult};

mod help;

pub use help::{error_output, exit, help, scoped_help};

/// 命令异步处理结果。
pub type CommandFuture<'a> = Pin<Box<dyn Future<Output = CommandResult> + Send + 'a>>;

/// 命令处理函数。
pub type CommandHandler = for<'a> fn(CommandContext<'a>, &'a [&'a str]) -> CommandFuture<'a>;

/// 命令处理结果。
pub type CommandResult = HyperResult<CommandOutput>;

/// 命令可用范围。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommandScope {
    /// 允许交互式控制台调用。
    Console,
    /// 保留给旧宏调用的兼容值,实际等同于 `Console`。
    Both,
}

impl CommandScope {
    /// 判断是否允许交互式控制台调用。
    pub fn allows_console(self) -> bool {
        matches!(self, Self::Console | Self::Both)
    }
}

/// 参数补全类型。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    /// 不补全参数。
    None,
    /// 从当前配置版本列表补全。
    Version,
}

/// 命令参数策略。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CommandArguments {
    /// 命令不接受额外参数。
    None,
    /// 命令自行解析额外参数。
    Any,
}

/// 参数补全提供器。
pub trait CompletionProvider: Send + Sync {
    /// 返回指定补全类型的候选值。
    fn complete(&self, kind: CompletionKind, prefix: &str) -> Vec<String>;
}

/// 命令运行上下文。
#[derive(Clone, Copy)]
pub struct CommandContext<'a> {
    /// 当前命令注册表。
    pub registry: &'a CommandRegistry,
    /// 调用方注入的业务状态。
    pub state: &'a (dyn Any + Send + Sync),
}

impl<'a> CommandContext<'a> {
    /// 读取调用方注入的业务状态。
    pub fn state<T: 'static>(&self) -> HyperResult<&'a T> {
        self.state
            .downcast_ref::<T>()
            .ok_or_else(|| HyperError::new("command state type mismatch"))
    }
}

/// 已注册命令。
#[derive(Clone, Copy)]
pub struct RegisteredCommand {
    /// 命令路径。
    pub path: &'static [&'static str],
    /// 帮助输出中的用法。
    pub usage: &'static str,
    /// 命令说明。
    pub description: &'static str,
    /// 命令可用范围。
    pub scope: CommandScope,
    /// 参数补全类型。
    pub completion: CompletionKind,
    /// 命令参数策略。
    pub arguments: CommandArguments,
    /// 是否显示在常规帮助里。
    pub visible: bool,
    /// 命令处理函数。
    pub handler: CommandHandler,
}

/// 命令执行结果。
pub struct CommandOutput {
    /// 命令执行摘要。
    pub summary: String,
    /// 控制台输出文本。
    pub rendered: String,
}

/// 命令注册表。
#[derive(Clone)]
pub struct CommandRegistry {
    /// 已注册命令。
    commands: Arc<[RegisteredCommand]>,
}

impl CommandRegistry {
    /// 从静态命令列表创建命令注册表。
    pub fn new(commands: &'static [RegisteredCommand]) -> Self {
        Self {
            commands: Arc::from(commands),
        }
    }

    /// 从已构造的命令列表创建命令注册表。
    pub fn from_commands(commands: Vec<RegisteredCommand>) -> Self {
        Self {
            commands: Arc::from(commands),
        }
    }

    /// 返回当前注册的全部命令。
    pub fn commands(&self) -> &[RegisteredCommand] {
        &self.commands
    }

    /// 执行交互式控制台命令。
    pub async fn execute_console(
        &self,
        command: &str,
        state: &(dyn Any + Send + Sync),
    ) -> HyperResult<CommandOutput> {
        self.execute(command, state, CommandScope::Console).await
    }

    /// 解析命令、校验调用范围和参数策略,再交给 handler 执行。
    async fn execute<'a>(
        &'a self,
        command: &str,
        state: &'a (dyn Any + Send + Sync),
        scope: CommandScope,
    ) -> HyperResult<CommandOutput> {
        let parts = parse_parts(command)?;
        let entry = self.resolve(&parts, scope)?;
        let args = &parts[entry.path.len()..];
        validate_arguments(entry, args)?;
        (entry.handler)(command_context(self, state), args).await
    }

    /// 根据当前输入返回基础补全候选。
    pub fn complete(&self, input: &str, provider: &dyn CompletionProvider) -> Vec<String> {
        let ends_with_space = input.chars().last().is_some_and(char::is_whitespace);
        let tokens = input.split_whitespace().collect::<Vec<_>>();
        let (base, prefix) = if ends_with_space {
            (tokens.as_slice(), "")
        } else {
            let split_at = tokens.len().saturating_sub(1);
            (&tokens[..split_at], tokens.last().copied().unwrap_or(""))
        };
        let mut values = BTreeSet::new();
        self.collect_path_completions(base, prefix, &mut values);
        self.collect_argument_completions(base, prefix, provider, &mut values);
        values.into_iter().collect()
    }

    /// 生成命令帮助输出。
    pub fn help_output(&self, args: &[&str]) -> String {
        help::command_help(self.commands(), args)
    }

    /// 按最长前缀匹配命令,让 `version switch v2` 优先命中 `version switch`。
    fn resolve(&self, parts: &[&str], scope: CommandScope) -> HyperResult<&RegisteredCommand> {
        let mut matched: Option<&RegisteredCommand> = None;
        for command in self.commands() {
            if scope == CommandScope::Console && !command.scope.allows_console() {
                continue;
            }
            if parts.len() < command.path.len() {
                continue;
            }
            if command.path.iter().zip(parts.iter()).all(|(a, b)| a == b) {
                matched = match matched {
                    Some(previous) if previous.path.len() >= command.path.len() => Some(previous),
                    _ => Some(command),
                };
            }
        }
        matched.ok_or_else(|| HyperError::new(format!("unknown command: {}", parts.join(" "))))
    }

    /// 收集当前输入位置可能出现的下一级命令路径。
    fn collect_path_completions(&self, base: &[&str], prefix: &str, values: &mut BTreeSet<String>) {
        for command in self.commands() {
            if !command.scope.allows_console() {
                continue;
            }
            if command.path.len() <= base.len() {
                continue;
            }
            if !command.path.iter().zip(base.iter()).all(|(a, b)| a == b) {
                continue;
            }
            let candidate = command.path[base.len()];
            if candidate.starts_with(prefix) {
                values.insert(candidate.to_owned());
            }
        }
    }

    /// 当输入已经完整匹配某个命令路径时,收集该命令的参数补全候选。
    fn collect_argument_completions(
        &self,
        base: &[&str],
        prefix: &str,
        provider: &dyn CompletionProvider,
        values: &mut BTreeSet<String>,
    ) {
        let Some(command) = self.find_exact(base) else {
            return;
        };
        for value in provider.complete(command.completion, prefix) {
            values.insert(value);
        }
    }

    /// 查找和当前输入完全一致的命令路径,用于决定是否进入参数补全阶段。
    fn find_exact(&self, parts: &[&str]) -> Option<&RegisteredCommand> {
        self.commands()
            .iter()
            .find(|command| command.path == parts && command.scope.allows_console())
    }
}

/// 执行命令自身声明的参数策略。
fn validate_arguments(command: &RegisteredCommand, args: &[&str]) -> HyperResult<()> {
    if command.arguments == CommandArguments::None && !args.is_empty() {
        return Err(HyperError::new("command does not accept arguments"));
    }
    Ok(())
}

/// 构造传给 handler 的轻量上下文。
fn command_context<'a>(
    registry: &'a CommandRegistry,
    state: &'a (dyn Any + Send + Sync),
) -> CommandContext<'a> {
    CommandContext { registry, state }
}

/// 将用户输入拆成命令 token。空输入在进入命令分发前直接失败。
fn parse_parts(command: &str) -> HyperResult<Vec<&str>> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    if parts.is_empty() {
        return Err(HyperError::new("empty command"));
    }
    Ok(parts)
}
