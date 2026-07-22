//! 可复用 gateway 运行器。调用方负责提供运行配置。

use std::sync::Arc;

use crate::control::GatewayControl;
use crate::http::{
    DefaultRequestKindClassifier, Gateway, HttpState, ProxyBodyPolicy, VersionClients,
};
use crate::management::{ManagementConfig, ManagementState};
use crate::runtime::VersionRegistry;
use hypergate_config::{
    ConfigManager, ConfigValidatorChain, DefaultConfigValidator, RuntimeConfig, StaticConfigLoader,
};
use hypergate_core::{HyperError, HyperResult};

use hypergate_cli::command::CommandRegistry;
use hypergate_cli::console::{CommandState, ConsoleOptions, control_loop, spawn_console};
use hypergate_cli::format::colorize_output;

use crate::commands::{self, GatewayCommandState, GatewayCompletionProvider};
use crate::views::start_output;

/// 根据外层参数执行启动或帮助。
pub(crate) async fn run(args: Vec<String>, config: RuntimeConfig) -> HyperResult<()> {
    let registry = CommandRegistry::new(commands::COMMANDS);
    let Some(command) = args.first().map(String::as_str) else {
        return run_app(config, registry).await;
    };
    if command == "start" {
        return run_app(config, registry).await;
    }
    if matches!(command, "help" | "-h" | "--help") {
        print_help(registry, &args[1..]);
        return Ok(());
    }
    Err(HyperError::new(
        "gateway commands are available in the running console",
    ))
}

/// 启动 HyperGate 和同进程控制台。
async fn run_app(config: RuntimeConfig, registry: CommandRegistry) -> HyperResult<()> {
    let listen = config.server.listen;
    // 管理配置在启动其他线程前校验,避免错误配置留下孤立控制台。
    let admin = ManagementConfig::from_env()?;
    let loader = Arc::new(StaticConfigLoader {
        template: config.clone(),
    });
    let mut validator_chain = ConfigValidatorChain::<RuntimeConfig>::new();
    validator_chain.push(Arc::new(DefaultConfigValidator));
    let validator = Arc::new(validator_chain);
    let manager = Arc::new(ConfigManager::new(config.clone(), loader, validator));
    let versions = Arc::new(VersionRegistry::new());

    for version_id in config.versions.keys() {
        versions.ensure(version_id.clone())?;
    }

    // 启动阶段只激活配置中的 active 版本,其他版本等待后续切换接管。
    let active = versions
        .get(&config.active_version)?
        .ok_or_else(|| HyperError::new("active version is not registered"))?;
    active.activate();

    let gateway = Arc::new(Gateway::new(&config, versions.clone())?);
    let control = Arc::new(GatewayControl::new(
        manager.clone(),
        gateway.clone(),
        versions.clone(),
    ));
    let state = HttpState {
        gateway: gateway.clone(),
        clients: VersionClients::new(),
        classifier: Arc::new(DefaultRequestKindClassifier),
        body_policy: ProxyBodyPolicy::default(),
    };
    let (console_tx, console_rx) = std::sync::mpsc::channel();

    let command_state: CommandState = Arc::new(GatewayCommandState {
        manager: manager.clone(),
        versions: versions.clone(),
        control: control.clone(),
    });
    let completion_provider = Arc::new(GatewayCompletionProvider {
        manager: manager.clone(),
    });
    let banner = colorize_output(&start_output(listen, &config));
    let registry = Arc::new(registry);
    spawn_console(
        console_tx,
        completion_provider,
        registry.clone(),
        ConsoleOptions::gateway(banner),
    );
    // HTTP 服务和控制循环共享同一套配置快照与版本运行态。
    tokio::spawn(control_loop(registry, command_state, console_rx));

    let proxy_future = crate::http::serve(state, listen);
    match admin {
        Some(admin_config) => {
            let admin_listen = admin_config.listen();
            let admin_credential = admin_config.into_credential();
            let admin_state = ManagementState::new(control, admin_credential);
            let admin_future = crate::management::serve(admin_state, admin_listen);
            // 管理 listener 与代理 listener 同生命周期,任一失败都让 run_app 返回错误。
            tokio::select! {
                result = proxy_future => result,
                result = admin_future => result,
            }
        }
        None => proxy_future.await,
    }
}

/// 打印本地命令帮助。
fn print_help(registry: CommandRegistry, args: &[String]) {
    let path = args.iter().map(String::as_str).collect::<Vec<_>>();
    println!("{}", colorize_output(&registry.help_output(&path)));
}
