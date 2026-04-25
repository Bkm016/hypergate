# hypergate-app

`hypergate-app` 是业务 version app 的 SDK。它让开发者在自己的项目里组装 HTTP handler、业务配置和控制台指令，再用统一入口启动一个独立业务进程。

Version app 是独立可执行文件。它监听自己的端口，gateway 只通过配置把某个 version id 映射到这个 endpoint。Version app 不接收 `--version`，也不修改 gateway 的 active version。

## Boundary

| 允许 | 禁止 |
|---|---|
| 解析 version app 自身启动参数 | 解析 gateway version id |
| 启动 Axum 服务 | 启动 gateway |
| 接入 version app 控制台 | 控制 gateway 切换 |
| 注册业务指令 | 托管其他 version 进程 |
| 注入业务状态 | 写入控制文件或监听控制端口 |

## Modules

| 模块 | 文件 | 职责 |
|---|---|---|
| `app` | `src/app.rs` | `VersionApp` 运行器 |
| `console` | `src/console.rs` | `VersionAppConsole`、builder、默认命令 |
| `options` | `src/options.rs` | `VersionAppOptions` 启动参数 |

## Startup

最小入口：

```rust
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use hypergate_core::HyperResult;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("version failed: {error}");
        std::process::exit(1);
    }
}

async fn run() -> HyperResult<()> {
    hypergate_version_app::run(SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::LOCALHOST,
        9101,
    )))
    .await
}
```

`VersionApp::run_from_env(default_listen, build)` 只支持 version app 自身参数：

| 参数 | 说明 |
|---|---|
| 无参数 | 使用代码传入的默认监听地址 |
| `--listen <addr>` | 覆盖监听地址 |
| `--help` / `help` | 打印 version app 启动帮助 |

## Console SDK

| API | 用途 |
|---|---|
| `VersionAppConsole::default_for` | 接入 SDK 默认控制台 |
| `VersionAppConsole::builder` | 在默认命令基础上追加业务指令 |
| `VersionAppCommandState<T>` | 把 `VersionAppOptions` 和业务状态注入 handler |
| `VersionAppConsoleBuilder::state` | 设置业务状态类型 |
| `VersionAppConsoleBuilder::commands` | 批量注册业务命令 |

SDK 默认命令：

| 指令 | 作用 |
|---|---|
| `show status` | 查看当前业务进程名称和监听地址 |
| `help [path]` | 查看命令树 |
| `exit` | 输出停止提示 |

业务指令示例见项目根 `src/commands.rs`。

## Business Config

Version app 可以直接复用 `hypergate-config` 的泛型快照层：

| 场景 | 推荐 API |
|---|---|
| 简单内存配置 | `ConfigManager::static_config` |
| 自定义加载来源 | `ConfigLoader<T>` |
| 自定义校验 | `ConfigValidator<T>` 或闭包 |
| 命令触发重载 | `ConfigManager::reload(ReloadTrigger::Command { ... })` |

## Audit Checklist

| 项 | 要求 |
|---|---|
| 版本身份 | 业务进程名来自可执行文件名，不等同于 gateway version id |
| 启动参数 | 不支持 `--version` |
| 控制台 | 只控制当前业务进程 |
| 扩展方式 | 业务通过 builder 和宏注册命令，不修改 SDK crate |
| 依赖方向 | 可以依赖 `hypergate-cli` 和 `hypergate-core`，不能依赖 `hypergate` gateway crate |
