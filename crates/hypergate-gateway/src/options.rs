//! Gateway 进程启动参数。

use std::path::PathBuf;

use hypergate_core::{HyperError, HyperResult};

const DEFAULT_CONFIG_PATH: &str = "hypergate.toml";
const DEFAULT_STATE_PATH: &str = "hypergate.state.json";

/// Gateway 启动模式。
pub(crate) enum GatewayMode {
    /// 启动代理和管理服务。
    Start,
    /// 只校验配置后退出。
    Check,
    /// 输出帮助后退出。
    Help,
}

/// Gateway 命令行参数。
pub(crate) struct GatewayOptions {
    /// 本次执行模式。
    pub(crate) mode: GatewayMode,
    /// TOML 声明配置路径。
    pub(crate) config_path: PathBuf,
    /// Gateway 私有运行状态路径。
    pub(crate) state_path: PathBuf,
}

impl GatewayOptions {
    /// 解析 `start`、`check` 及配置路径参数。
    pub(crate) fn parse(args: Vec<String>) -> HyperResult<Self> {
        let mut values = args.into_iter();
        let mut mode = GatewayMode::Start;
        let mut config_path = PathBuf::from(DEFAULT_CONFIG_PATH);
        let mut state_path = PathBuf::from(DEFAULT_STATE_PATH);
        if let Some(first) = values.next() {
            match first.as_str() {
                "start" => {}
                "check" => mode = GatewayMode::Check,
                "help" | "-h" | "--help" => mode = GatewayMode::Help,
                value if value.starts_with('-') => {
                    values = std::iter::once(first)
                        .chain(values)
                        .collect::<Vec<_>>()
                        .into_iter();
                }
                _ => return Err(HyperError::new("expected start, check, or help")),
            }
        }
        while let Some(argument) = values.next() {
            match argument.as_str() {
                "--config" => {
                    config_path = PathBuf::from(
                        values
                            .next()
                            .ok_or_else(|| HyperError::new("--config requires a path"))?,
                    );
                }
                "--state" if matches!(mode, GatewayMode::Start) => {
                    state_path = PathBuf::from(
                        values
                            .next()
                            .ok_or_else(|| HyperError::new("--state requires a path"))?,
                    );
                }
                _ => return Err(HyperError::new(format!("unknown argument: {argument}"))),
            }
        }
        Ok(Self {
            mode,
            config_path,
            state_path,
        })
    }

    /// 输出稳定的命令行帮助。
    pub(crate) fn print_help() {
        println!("HyperGate gateway");
        println!("  hypergate start [--config <path>] [--state <path>]");
        println!("  hypergate check [--config <path>]");
    }
}
