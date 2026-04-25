//! Version app 启动参数。
//!
//! 该模块只解析业务进程自身需要的参数。Gateway 的 active version
//! 和切换策略不从这里读取,避免业务进程反向耦合控制面。
//!
//! @author sky

use std::net::SocketAddr;

use hypergate_core::{HyperError, HyperResult};

/// Version app 启动参数。
///
/// 每个 version app 都是独立业务进程,只需要知道自己的进程名和监听
/// 端口。Gateway 后续用自己的部署配置把 `v1` / `v2` 映射到这些端口。
#[derive(Debug, Clone)]
pub struct VersionAppOptions {
    /// 当前业务进程名称。
    ///
    /// 默认来自当前可执行文件名,例如 `hypergate-version-v1` 或
    /// `hypergate-version-v2`。该名称只用于控制台提示,
    /// 不等同于 gateway 配置里的 version id。
    pub name: String,
    /// 当前 version 进程监听地址。
    ///
    /// Gateway 会把命中的 active version 请求转发到该地址。生产环境
    /// 下不同 version 应监听不同端口。
    pub listen: SocketAddr,
}

impl VersionAppOptions {
    /// 从进程参数解析 version app 启动参数。
    ///
    /// 当前只支持可选的 `--listen <addr>`。gateway 的版本号只存在于
    /// gateway 部署配置中。
    pub fn from_env(default_listen: SocketAddr) -> HyperResult<Self> {
        Self::parse(std::env::args().skip(1), default_listen)
    }

    /// 从参数迭代器解析 version app 启动参数。
    ///
    /// 该方法用于测试或自定义启动器。未知参数会直接报错,避免静默
    /// 忽略拼写错误后启动到错误端口。
    pub fn parse<I>(args: I, default_listen: SocketAddr) -> HyperResult<Self>
    where
        I: IntoIterator<Item = String>,
    {
        let mut listen = None;
        let mut iter = args.into_iter();

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--listen" => {
                    listen = Some(
                        iter.next()
                            .ok_or_else(|| HyperError::new("missing --listen value"))?,
                    );
                }
                value => {
                    return Err(HyperError::new(format!(
                        "unknown version app argument: {value}"
                    )));
                }
            }
        }

        let listen = match listen {
            Some(value) => value
                .parse::<SocketAddr>()
                .map_err(|e| HyperError::new(format!("invalid --listen value: {e}")))?,
            None => default_listen,
        };

        Ok(Self {
            name: app_name_from_exe(),
            listen,
        })
    }
}

/// 从当前可执行文件名推导业务进程名称。
fn app_name_from_exe() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_stem()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "hypergate-version-v1".to_owned())
}
