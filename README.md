# HyperGate

HyperGate 是一个原生支持多版本部署的网络后端框架底座。它把稳定不变的 gateway 和可独立发布的 version app 分开：gateway 负责接收客户端请求、维护 active version、执行切换和排水；业务代码运行在独立 version app 可执行文件中，并监听自己的端口。

项目目标是让开发者在不修改框架 crates 的前提下开发业务服务，同时获得本地控制台、多版本切换、配置快照、长连接排水和高性能 HTTP 转发能力。

## Architecture

| 部分 | 产物 | 职责 | 不负责 |
|---|---|---|---|
| Gateway | `hypergate` | 对外监听、转发请求、切换 active version、记录版本连接状态 | 启动业务进程、进入业务控制台、解释业务路径 |
| Version app | `hypergate-version-v1`、`hypergate-version-v2` | 承载业务 HTTP 能力、业务配置、业务控制台指令 | 读取 gateway version id、修改 gateway active version |
| Shared crates | `hypergate-core`、`hypergate-config`、`hypergate-cli`、`hypergate-app` | 提供通用类型、配置快照、命令系统和 version app SDK | 携带 gateway 专属指令或业务逻辑 |

Gateway 配置中的 `v1`、`v2` 是部署标识，只存在于 gateway 控制面。Version app 不接收 `--version`，也不需要知道自己在 gateway 配置里叫 `v1` 还是 `v2`。

## Workspace

| 路径 | 发布身份 | 内容 |
|---|---|---|
| `crates/hypergate-gateway` | `hypergate` | Gateway 可执行文件，包含 HTTP 转发、版本运行态和 gateway 控制台指令 |
| `crates/hypergate-app` | `hypergate-app` | Version app SDK，封装启动、监听、控制台接入 |
| `crates/hypergate-core` | `hypergate-core` | 错误、请求类型、版本状态、扩展注册基础类型 |
| `crates/hypergate-config` | `hypergate-config` | 泛型配置快照、加载器、校验链和 gateway 运行配置 schema |
| `crates/hypergate-cli` | `hypergate-cli` | 通用命令树、补全、控制台和输出格式 |
| `src/` | `hypergate-version-app` 示例 | 官方 version app 示例，按 `config`、`commands`、HTTP handler 分层 |
| `scripts/e2e-ab.js` | 测试脚本 | 启动 gateway / v1 / v2，验证切换、回滚和控制台隔离 |

## Quick Start

启动两个 version app：

```bat
start-v1.bat
start-v2.bat
```

启动 gateway：

```bat
start-gateway.bat
```

访问 gateway：

```text
http://127.0.0.1:8080/
```

默认请求会转发到 gateway 配置中的 active version。当前示例默认 active version 是 `v1`。

## Console Commands

Gateway 和 version app 各自拥有独立控制台。指令只能在对应进程的运行中控制台输入，不走文件通道、控制端口或 HTTP 管理 API。

| 进程 | 指令 | 作用 |
|---|---|---|
| Gateway | `show status` | 查看 gateway 状态、active version 和版本连接计数 |
| Gateway | `show versions` | 查看 version endpoint、状态、请求数和流式连接数 |
| Gateway | `config` | 查看当前 gateway 运行配置 |
| Gateway | `config check` | 校验当前 gateway 运行配置 |
| Gateway | `config reload` | 重载配置快照并同步 active target |
| Gateway | `version switch <version>` | 把新请求切到目标 version |
| Gateway | `version rollback` | 切回上一个 active version |
| Gateway | `version drain <version>` | 让非 active version 停止接收新请求 |
| Gateway | `version stop <version>` | 在连接清空后停止非 active version |
| Version app | `show status` | 查看当前业务进程名称和监听地址 |
| Version app | `app info` | 示例业务信息 |
| Version app | `app config` | 示例业务配置快照 |
| Version app | `app reload` | 示例业务配置重载 |

`help` 会从命令注册表生成树状帮助。Tab 补全同样来自命令树和调用方提供的补全器。

## Multi Version Flow

```text
client
  -> hypergate:8080
  -> active version snapshot
  -> version lease
  -> selected version app endpoint
  -> response stream
  -> lease release
```

切换流程：

```text
version switch v2
  -> v2 becomes active
  -> previous active version enters draining
  -> config snapshot is replaced
  -> gateway active target is refreshed
  -> new requests go to v2
  -> old streams keep their original lease
```

旧连接不会因为 active version 切换被强制打断。响应流持有 `VersionLease`，流结束或出错时自动释放计数。

## Development Model

开发者通常只写自己的 version app：

| 文件 | 建议职责 |
|---|---|
| `src/bin/<app>.rs` | 可执行入口，只传入默认监听地址 |
| `src/lib.rs` | 组装配置、HTTP handler 和控制台 |
| `src/config.rs` | 业务配置类型、校验和配置管理器 |
| `src/commands.rs` | 业务控制台指令注册 |
| `src/routes.rs` | 业务 HTTP handler 示例 |

业务指令通过 `hypergate_command!` / `hypergate_commands!` 注册，业务配置通过 `ConfigManager<T>` 接入。开发者不应为了扩展业务能力修改 `crates/` 里的框架代码。

## Release Gates

| 检查 | 命令 |
|---|---|
| Rust 类型和文档 lint | `cargo check --workspace` |
| Clippy | `cargo clippy --workspace --all-targets` |
| Rustdoc | `cargo doc --workspace --no-deps` |
| 端到端 AB 切换 | `node scripts/e2e-ab.js` |
| 格式检查 | `git diff --check` |

## Current Scope

| 能力 | 状态 |
|---|---|
| HTTP 转发 | 已实现 |
| 多版本切换和回滚 | 已实现 |
| 长连接租约计数 | 已实现 |
| Gateway 控制台 | 已实现 |
| Version app 控制台 SDK | 已实现 |
| 泛型配置快照 | 已实现 |
| 文件监听 loader | 未实现 |
| WebSocket 隧道 | 未实现 |
| Runtime metrics | 未实现 |

## Design Constraints

| 约束 | 说明 |
|---|---|
| 不内置 HTTP 管理 API | 控制面只走运行中进程控制台 |
| 不使用控制文件或控制端口 | 不保留 `.hypergate/control.*`、`control.json` 或额外控制监听 |
| Gateway 不托管业务进程 | 业务进程由部署系统或用户自行启动 |
| Version app 不知道 gateway version id | 部署标识只存在于 gateway 配置 |
| 请求热路径不读写配置锁 | Active target 用快照替换 |
| 框架 crates 保持通用 | 只有 `hypergate-gateway` 可以放 gateway 专属逻辑 |
