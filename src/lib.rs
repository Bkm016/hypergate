//! HyperGate 官方 version app 示例。
//!
//! 这里展示业务开发者如何在项目 `src/` 中组织自己的 version app。
//! `config`、`commands`、`routes` 分别承载配置、控制台指令和 HTTP
//! 路由,可执行入口只负责传入默认监听端口。
//!
//! @author sky

mod commands;
mod config;
mod routes;

use std::net::SocketAddr;

use hypergate_app::{VersionApp, VersionAppOptions};
use hypergate_core::HyperResult;

use crate::commands::demo_console;
use crate::config::demo_config_handle;
use crate::routes::demo_router;

/// 使用指定默认监听地址启动 demo version app。
pub async fn run(default_listen: SocketAddr) -> HyperResult<()> {
    VersionApp::run_from_env(default_listen, build_app).await
}

/// 组装 demo version app 的配置、路由和控制台。
fn build_app(options: VersionAppOptions) -> VersionApp {
    let config = demo_config_handle();
    let router = demo_router(options.name.clone(), config.clone());
    let console = demo_console(&options, config);
    VersionApp::new(options, router).with_console(console)
}
