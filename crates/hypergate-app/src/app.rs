//! Version app 运行器。
//!
//! `VersionApp` 是业务进程的最外层启动封装。它不参与 gateway 的
//! 多版本切换决策,只负责按 `VersionAppOptions` 监听端口、启动
//! Axum 路由和可选的 version 控制台。
//!
//! @author sky

use std::net::SocketAddr;

use axum::Router;
use hypergate_core::{HyperError, HyperResult};

use crate::console::VersionAppConsole;
use crate::options::VersionAppOptions;

/// Version app 运行器。
///
/// 开发者把业务 `Router` 交给该类型,再选择是否接入默认控制台或
/// 自定义控制台。该类型只属于 version 进程,不会启动 gateway,也不会
/// 修改 gateway 的 active version。
pub struct VersionApp {
    /// 启动参数。
    options: VersionAppOptions,
    /// 用户业务路由。
    router: Router,
    /// Version app 控制台配置。
    console: Option<VersionAppConsole>,
}

impl VersionApp {
    /// 从进程参数启动 version app。
    ///
    /// 业务入口推荐使用该方法。空参数使用默认监听地址,`--listen <addr>`
    /// 覆盖监听地址。业务控制指令在运行中的控制台输入。
    pub async fn run_from_env<F>(default_listen: SocketAddr, build: F) -> HyperResult<()>
    where
        F: FnOnce(VersionAppOptions) -> Self,
    {
        Self::try_run_from_env(default_listen, |options| Ok(build(options))).await
    }

    /// 从进程参数启动可能失败的 version app 构造流程。
    ///
    /// 当业务需要在构造路由或控制台时读取外部资源,并且该过程可能失败,
    /// 使用该方法把错误交回统一启动入口处理。
    pub async fn try_run_from_env<F>(default_listen: SocketAddr, build: F) -> HyperResult<()>
    where
        F: FnOnce(VersionAppOptions) -> HyperResult<Self>,
    {
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        if should_print_help(&args) {
            print_version_app_help(default_listen);
            return Ok(());
        }
        let options = VersionAppOptions::parse(args, default_listen)?;
        build(options)?.run().await
    }

    /// 创建 version app 运行器。
    ///
    /// `options` 描述当前 version 的身份和监听端口,`router` 是业务
    /// HTTP 入口。默认不会自动启用控制台,需要显式调用
    /// `with_default_console` 或 `with_console`。
    pub fn new(options: VersionAppOptions, router: Router) -> Self {
        Self {
            options,
            router,
            console: None,
        }
    }

    /// 接入默认 version app 控制台。
    ///
    /// 默认控制台包含 `show status`、`help` 和 `exit`。如果业务需要
    /// 注册自己的指令,优先使用 `VersionAppConsole::builder` 构造后
    /// 再传给 `with_console`。
    pub fn with_default_console(mut self) -> Self {
        self.console = Some(VersionAppConsole::default_for(&self.options));
        self
    }

    /// 接入自定义 version app 控制台。
    ///
    /// 自定义控制台用于追加业务指令、替换补全来源或调整控制台选项。
    /// SDK 不要求业务修改 `hypergate-app` crate 本身。
    pub fn with_console(mut self, console: VersionAppConsole) -> Self {
        self.console = Some(console);
        self
    }

    /// 返回当前业务进程名称。
    ///
    /// 该值来自当前可执行文件名,通常用于日志和控制台输出。
    pub fn name(&self) -> &str {
        &self.options.name
    }

    /// 启动当前 version app。
    ///
    /// 启动顺序是先启动控制台,再绑定 HTTP 监听端口。控制台失败会让
    /// 进程启动失败,避免业务已经对外服务但交互控制台不可用。
    pub async fn run(self) -> HyperResult<()> {
        println!(
            "{} listening on http://{}",
            self.options.name, self.options.listen
        );
        if let Some(console) = self.console {
            console.spawn().await?;
        }
        let listener = tokio::net::TcpListener::bind(self.options.listen)
            .await
            .map_err(|e| HyperError::new(format!("bind version failed: {e}")))?;
        axum::serve(listener, self.router)
            .await
            .map_err(|e| HyperError::new(format!("serve version failed: {e}")))
    }
}

/// 判断当前启动参数是否只请求帮助信息。
fn should_print_help(args: &[String]) -> bool {
    matches!(args, [value] if value == "-h" || value == "--help" || value == "help")
}

/// 输出 version app 的最小启动帮助。
fn print_version_app_help(default_listen: SocketAddr) {
    println!("HyperGate version app");
    println!("  start with default listen: {default_listen}");
    println!("  override listen: --listen <addr>");
    println!("  commands are available in the running console");
}
