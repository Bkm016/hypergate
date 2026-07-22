# hypergate

`hypergate` 是 HyperGate 的 gateway 可执行 crate。它是最小不变单元：对外监听 HTTP 请求，维护 active version，执行多版本切换、回滚和排水，并把请求转发到当前 active version endpoint。

Gateway 不启动业务进程，也不进入业务控制台。业务 version app 由用户或部署系统自行启动。

## Boundary

| 允许 | 禁止 |
|---|---|
| 对外监听 HTTP | 启动 version app 进程 |
| 转发请求到 active version endpoint | 读取业务配置 |
| 切换 active version | 执行业务控制台指令 |
| 跟踪请求和流式连接计数 | 暴露远程文件控制通道 |
| Gateway 控制台指令 | 未鉴权的控制通道 |
| 本机管理 HTTP API(默认关闭) | 远程或公网管理监听 |

## Modules

| 模块 | 文件或目录 | 职责 |
|---|---|---|
| `app` | `src/app.rs` | Gateway 启动、控制台、HTTP 服务和管理服务组装 |
| `commands` | `src/commands/` | Gateway 专属控制台指令 |
| `control` | `src/control.rs` | 生命周期控制入口,CLI 与 HTTP API 共享 |
| `default_config` | `src/default_config.rs` | 本地示例配置 |
| `http` | `src/http/` | HTTP 服务、转发、请求体限制、响应流租约 |
| `management` | `src/management.rs` | 本机管理 HTTP 服务、Bearer 鉴权和路由 |
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

## Management API

管理 API 默认关闭。仅在设置环境变量 `HYPERGATE_ADMIN_TOKEN` 后启用,所有路由要求 `Authorization: Bearer <token>` 鉴权。

| 环境变量 | 作用 | 默认值 |
|---|---|---|
| `HYPERGATE_ADMIN_TOKEN` | 管理 API 鉴权 token。未设置时管理 API 禁用 | 无 |
| `HYPERGATE_ADMIN_LISTEN` | 管理 API 监听地址,必须是 loopback 地址 | `127.0.0.1:8090` |

监听地址非 loopback 时拒绝启动。Token 必须由可见 ASCII 字符组成且不超过 1024 字节。不配置 CORS。

| 路由 | 方法 | 作用 |
|---|---|---|
| `/api/v1/status` | GET | 返回当前 gateway 状态快照 |
| `/api/v1/actions/switch` | POST | 切换 active version |
| `/api/v1/actions/drain` | POST | 让非 active 版本进入 draining |
| `/api/v1/actions/stop` | POST | 在连接清空后停止非 active 版本 |
| `/api/v1/actions/rollback` | POST | 切回上一个 active version |

`switch` / `drain` / `stop` 请求体包含 `version` 和必填的 `expectedRevision`;`rollback` 请求体只包含必填的 `expectedRevision`。`expectedRevision` 用于乐观并发控制,与当前配置修订号不匹配时返回 409。动作成功后直接返回最新状态快照。

错误响应为稳定 JSON:

| 状态码 | 含义 |
|---|---|
| 400 | 非法请求(缺少字段、非法 JSON、空版本标识) |
| 401 | 缺失或非法鉴权 |
| 409 | 控制动作被当前运行态拒绝 |
| 404 / 405 | 路由不存在或方法不支持 |
| 500 | 内部控制或状态快照错误 |

动作已经生效但最新快照读取失败时返回 500,错误消息为 `control action applied; status unavailable`,客户端不得自动重试该动作。

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
| 控制通道 | 不创建 `.hypergate`、`control.json` 等文件控制通道 |
| 管理 API | 默认关闭,仅本机 loopback 监听,Bearer 鉴权 |
| 转发语义 | 保留 method、path、query、body 和端到端 header |
| Header 处理 | 丢弃逐跳 header 和 framing header |
| 切换顺序 | 预构建目标、激活新版本、原子切流、提交配置，再排水旧版本 |
| 回滚历史 | 只记录实际切换前的 active version |
