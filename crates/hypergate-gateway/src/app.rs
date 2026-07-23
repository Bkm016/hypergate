//! 可复用 gateway 运行器。调用方负责提供运行配置。

use std::sync::Arc;

use crate::control::GatewayControl;
use crate::http::{
    DefaultRequestKindClassifier, Gateway, HealthChecker, HttpState, ProxyBodyPolicy,
    VersionClients,
};
use crate::management::{ManagementConfig, ManagementState};
use crate::options::{GatewayMode, GatewayOptions};
use crate::runtime::VersionRegistry;
use crate::state::StateStore;
use hypergate_config::{
    ConfigLoader, ConfigManager, ConfigValidator, ConfigValidatorChain, DefaultConfigValidator,
    RuntimeConfig, TomlConfigLoader,
};
use hypergate_core::{ConfigRevision, HyperError, HyperResult};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use hypergate_cli::command::CommandRegistry;
use hypergate_cli::console::{CommandState, ConsoleOptions, control_loop, spawn_console};
use hypergate_cli::format::colorize_output;

use crate::commands::{self, GatewayCommandState, GatewayCompletionProvider};
use crate::views::start_output;

/// 根据进程参数执行启动、配置校验或帮助。
pub(crate) async fn run(args: Vec<String>) -> HyperResult<()> {
    let options = GatewayOptions::parse(args)?;
    if matches!(options.mode, GatewayMode::Help) {
        GatewayOptions::print_help();
        return Ok(());
    }
    let loader = Arc::new(TomlConfigLoader::new(options.config_path));
    let mut validator_chain = ConfigValidatorChain::<RuntimeConfig>::new();
    validator_chain.push(Arc::new(DefaultConfigValidator));
    let validator = Arc::new(validator_chain);
    let mut config = loader.load(ConfigRevision::INITIAL)?;
    validator.validate(&config)?;
    if matches!(options.mode, GatewayMode::Check) {
        println!("configuration is valid");
        return Ok(());
    }
    let state_store = Arc::new(StateStore::new(options.state_path));
    let mut persisted = state_store.load_or_initialize(config.active_version.clone())?;
    if !config.versions.contains_key(&persisted.active_version) {
        return Err(HyperError::new(format!(
            "persisted active version is not configured: {}",
            persisted.active_version.value
        )));
    }
    persisted
        .history
        .retain(|version| config.versions.contains_key(version));
    state_store.save(&persisted)?;
    config.revision = persisted.revision;
    config.active_version = persisted.active_version.clone();
    validator.validate(&config)?;

    let registry = CommandRegistry::new(commands::COMMANDS);
    run_app(
        config,
        loader,
        validator,
        state_store,
        persisted.history,
        registry,
    )
    .await
}

/// 启动 HyperGate 和同进程控制台。
async fn run_app(
    config: RuntimeConfig,
    loader: Arc<dyn ConfigLoader<RuntimeConfig>>,
    validator: Arc<dyn ConfigValidator<RuntimeConfig>>,
    state_store: Arc<StateStore>,
    history: Vec<hypergate_core::VersionId>,
    registry: CommandRegistry,
) -> HyperResult<()> {
    let listen = config.server.listen;
    // 管理配置在启动其他线程前校验,避免错误配置留下孤立控制台。
    let admin = ManagementConfig::from_env()?;
    let manager = Arc::new(ConfigManager::with_revision(
        config.clone(),
        loader,
        validator,
        config.revision,
    ));
    let versions = Arc::new(VersionRegistry::new());

    for version_id in config.versions.keys() {
        versions.ensure(version_id.clone())?;
    }

    // 启动监听前确认持久化 active version 已可接收流量，失败时交给进程监管重试。
    let active_config = config.active_version_config()?;
    HealthChecker::new(
        config.server.version_connect_timeout,
        config.server.version_health_timeout,
    )
    .check(&active_config.health)
    .await?;
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
        state_store,
        history,
    ));
    let state = HttpState {
        gateway: gateway.clone(),
        // version app 连接超时直接由 server 配置驱动,避免重复配置源。
        clients: VersionClients::new(config.server.version_connect_timeout),
        classifier: Arc::new(DefaultRequestKindClassifier),
        body_policy: ProxyBodyPolicy::default(),
        server: config.server.clone(),
        tasks: TaskTracker::new(),
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

    let shutdown = CancellationToken::new();
    let signal_shutdown = shutdown.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        signal_shutdown.cancel();
    });
    let mut services = JoinSet::new();
    let proxy_shutdown = shutdown.clone();
    services.spawn(crate::http::serve(state, proxy_shutdown));
    if let Some(admin_config) = admin {
        let admin_listen = admin_config.listen();
        let admin_credential = admin_config.into_credential();
        let admin_state = ManagementState::new(control, admin_credential);
        let admin_shutdown = shutdown.clone();
        services.spawn(crate::management::serve(
            admin_state,
            admin_listen,
            admin_shutdown,
        ));
    }
    let mut failure = None;
    while let Some(result) = services.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) if failure.is_none() => failure = Some(error),
            Err(error) if failure.is_none() => {
                failure = Some(HyperError::new(format!(
                    "gateway service task failed: {error}"
                )));
            }
            _ => {}
        }
        shutdown.cancel();
    }
    match failure {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 等待 Ctrl+C 或 Unix SIGTERM。
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
