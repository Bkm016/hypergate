# hypergate

`hypergate` 是 HyperGate 的 gateway 可执行 crate。它是最小不变单元：对外监听 HTTP 请求，维护 active version，执行多版本切换、回滚和排水，并把请求转发到当前 active version endpoint。

Gateway 不启动业务进程，也不进入业务控制台。业务 version app 由用户或部署系统自行启动。

## Boundary

| 允许 | 禁止 |
|---|---|
| 对外监听 HTTP | 启动 version app 进程 |
| 转发请求到 active version endpoint | 读取业务配置 |
| 切换 active version | 执行业务控制台指令 |
| 跟踪请求和流式连接计数 | 暴露 HTTP 管理 API |
| Gateway 控制台指令 | 文件控制通道或控制端口 |

## Modules

| 模块 | 文件或目录 | 职责 |
|---|---|---|
| `app` | `src/app.rs` | Gateway 启动、控制台和 HTTP 服务组装 |
| `commands` | `src/commands/` | Gateway 专属控制台指令 |
| `default_config` | `src/default_config.rs` | 本地示例配置 |
| `http` | `src/http/` | HTTP 服务、转发、请求体限制、响应流租约 |
| `runtime` | `src/runtime/` | Version 运行态、租约、切换和排水 |
| `views` | `src/views.rs` | Gateway 控制台输出视图 |

## Default Runtime

默认配置用于本地开发：

| version | endpoint | health |
|---|---|---|
| `v1` | `http://127.0.0.1:9101` | `http://127.0.0.1:9101/health` |
| `v2` | `http://127.0.0.1:9102` | `http://127.0.0.1:9102/health` |

Gateway 对外监听：

```text
http://127.0.0.1:8080
```

## Commands

| 指令 | 作用 |
|---|---|
| `show status` | 查看 active version、endpoint、状态和连接计数 |
| `show versions` | 查看所有 version endpoint 和运行态 |
| `config` | 查看当前运行配置 |
| `config check` | 校验当前运行配置 |
| `config reload` | 重载配置快照并刷新 active target |
| `version switch <version>` | 把新请求切到目标 version |
| `version rollback` | 切回上一个 active version |
| `version drain <version>` | 让非 active version 停止接收新请求 |
| `version stop <version>` | 在连接清空后停止非 active version |
| `help [path]` | 查看命令树 |

外层 `hypergate help` 只打印帮助。运行态指令必须在 gateway 进程的控制台输入。

## Request Lifecycle

```text
accept connection
  -> classify request kind
  -> load active target snapshot
  -> create version lease
  -> rebuild URI with active endpoint
  -> stream request body with limit
  -> forward response body
  -> release lease when response stream ends
```

请求切换只影响新请求。已经持有租约的请求或流式连接继续绑定原 version。

## Performance Rules

| 热路径约束 | 实现 |
|---|---|
| 不在每个请求里解析 active endpoint | `Gateway` 持有预解析 active target snapshot |
| 不为大请求体做无上限聚合 | `LimitedBodyStream` 按 chunk 转发并限制总量 |
| 不用配置写锁服务请求 | 请求侧读取 `ArcSwap` 快照 |
| 长连接不被切换打断 | `LeaseStream` 持有 `VersionLease` |
| 普通请求和流式请求可分池 | `VersionClients` 按 `RequestKind` 选择 client |

## Audit Checklist

| 项 | 要求 |
|---|---|
| 指令归属 | Gateway 指令只在本 crate 注册 |
| 进程职责 | 不托管 version app |
| 控制通道 | 不创建 `.hypergate`、`control.json`、控制端口 |
| 转发语义 | 保留 method、path、query、body 和端到端 header |
| Header 处理 | 丢弃逐跳 header 和 framing header |
| 切换顺序 | 先切运行态，再提交配置快照并刷新 active target |
| 回滚历史 | 只记录实际切换前的 active version |
