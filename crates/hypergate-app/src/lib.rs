//! Version app SDK。
//!
//! 这个 crate 是业务 version app 的推荐入口。开发者在项目 `src/`
//! 中创建 HTTP 路由、注册 version 控制台指令,再交给 `VersionApp`
//! 统一启动。Gateway 只负责把流量切到某个 version endpoint,不会托管
//! 业务进程,所以 version app 的启动参数、监听地址和控制台都在这里收敛。
//!
//! @author sky

#![deny(missing_docs)]

pub use axum;
pub use hypergate_cli::command::{
    CommandArguments, CommandContext, CommandOutput, CommandScope, CompletionKind,
    CompletionProvider, RegisteredCommand,
};
pub use hypergate_cli::console::ConsoleOptions;
pub use hypergate_cli::format::{Align, Table, column, render_panel};
pub use hypergate_cli::{hypergate_command, hypergate_commands};

mod app;
mod console;
mod options;

pub use app::VersionApp;
pub use console::{VersionAppCommandState, VersionAppConsole, VersionAppConsoleBuilder};
pub use options::VersionAppOptions;
