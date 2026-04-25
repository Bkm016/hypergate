//! 同进程控制台。用户在启动进程的终端里直接输入指令。

use std::any::Any;
use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::Duration;

use rustyline::completion::{Completer, Pair};
use rustyline::config::CompletionType;
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};

use crate::command::{CommandRegistry, CompletionProvider, error_output};
use crate::format::{colorize_candidate, colorize_output, colorize_prompt};

/// 控制台输入事件。
pub struct ConsoleInput {
    /// 用户输入的命令。
    pub command: String,
    /// 命令处理完成通知。
    pub completed: Sender<()>,
}

/// 命令状态对象。
pub type CommandState = Arc<dyn Any + Send + Sync>;

/// 控制台运行选项。
#[derive(Debug, Clone)]
pub struct ConsoleOptions {
    /// 交互式提示符。
    pub prompt: String,
    /// 启动时输出的横幅。
    pub banner: String,
}

impl ConsoleOptions {
    /// 创建控制台运行选项。
    pub fn new(prompt: impl Into<String>, banner: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            banner: banner.into(),
        }
    }

    /// 创建 gateway 默认控制台选项。
    pub fn gateway(banner: impl Into<String>) -> Self {
        Self::new("hypergate> ", banner)
    }
}

/// 启动交互控制台线程。
pub fn spawn_console(
    sender: Sender<ConsoleInput>,
    completion_provider: Arc<dyn CompletionProvider>,
    registry: Arc<CommandRegistry>,
    options: ConsoleOptions,
) {
    thread::spawn(move || {
        if !plain_console_enabled()
            && run_interactive_console(sender.clone(), completion_provider, registry, &options)
                .is_ok()
        {
            return;
        }
        run_plain_console(sender, &options.prompt, options.banner);
    });
}

/// 使用标准输入输出运行兜底控制台。
fn run_plain_console(sender: Sender<ConsoleInput>, prompt: &str, banner: String) {
    if !banner.is_empty() {
        println!("{banner}");
    }
    let stdin = io::stdin();
    loop {
        print!("{prompt}");
        let _ = io::stdout().flush();
        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let command = line.trim();
                if !command.is_empty() && !send_console_command(&sender, command) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

/// 判断是否强制使用无补全、无行编辑的简单控制台。
fn plain_console_enabled() -> bool {
    std::env::var_os("HYPERGATE_PLAIN_CONSOLE").is_some()
}

/// rustyline 适配器,把通用命令注册表接入补全和高亮。
struct ConsoleHelper {
    /// 业务参数补全提供器。
    completion_provider: Arc<dyn CompletionProvider>,
    /// 当前进程的命令注册表。
    registry: Arc<CommandRegistry>,
}

impl Helper for ConsoleHelper {}

impl Completer for ConsoleHelper {
    type Candidate = Pair;

    /// 从注册表读取当前输入位置可用的补全候选。
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        let input = &line[..pos];
        let start = completion_start(input);
        let pairs = self
            .registry
            .complete(input, self.completion_provider.as_ref())
            .into_iter()
            .map(|value| Pair {
                display: value.clone(),
                replacement: format!("{value} "),
            })
            .collect();
        Ok((start, pairs))
    }
}

impl Hinter for ConsoleHelper {
    type Hint = String;
}

impl Highlighter for ConsoleHelper {
    /// 给交互提示符着色。
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        colorize_prompt(prompt)
    }

    /// 给补全候选着色。
    fn highlight_candidate<'c>(
        &self,
        candidate: &'c str,
        _completion: CompletionType,
    ) -> Cow<'c, str> {
        colorize_candidate(candidate)
    }
}

impl Validator for ConsoleHelper {}

/// 使用 rustyline 运行带历史、补全和基础高亮的控制台。
fn run_interactive_console(
    sender: Sender<ConsoleInput>,
    completion_provider: Arc<dyn CompletionProvider>,
    registry: Arc<CommandRegistry>,
    options: &ConsoleOptions,
) -> Result<(), ReadlineError> {
    let helper = ConsoleHelper {
        completion_provider,
        registry,
    };
    let mut editor = Editor::<ConsoleHelper, DefaultHistory>::new()?;
    editor.set_helper(Some(helper));
    if !options.banner.is_empty() {
        println!("{}", options.banner);
    }
    loop {
        match editor.readline(&options.prompt) {
            Ok(line) => {
                let command = line.trim();
                if command.is_empty() {
                    continue;
                }
                let _ = editor.add_history_entry(command);
                if !send_console_command(&sender, command) {
                    break;
                }
            }
            Err(ReadlineError::Interrupted | ReadlineError::Eof) => break,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// 把控制台输入送入异步控制循环,并等待本次命令输出完成。
fn send_console_command(sender: &Sender<ConsoleInput>, command: &str) -> bool {
    let (completed, wait) = std::sync::mpsc::channel();
    let input = ConsoleInput {
        command: command.to_owned(),
        completed,
    };
    if sender.send(input).is_err() {
        return false;
    }
    wait.recv().is_ok()
}

/// 返回当前 token 起始位置,用于让 rustyline 替换最后一个 token。
fn completion_start(input: &str) -> usize {
    input
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0)
}

/// 控制循环。交互式控制台输入进入同一套命令注册表。
pub async fn control_loop(
    registry: Arc<CommandRegistry>,
    state: CommandState,
    console_rx: Receiver<ConsoleInput>,
) {
    loop {
        while let Ok(input) = console_rx.try_recv() {
            let output = registry
                .execute_console(&input.command, state.as_ref())
                .unwrap_or_else(|error| error_output(&error.to_string()));
            println!("{}", colorize_output(&output.rendered));
            let _ = input.completed.send(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
