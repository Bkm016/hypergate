# hypergate-config

`hypergate-config` 提供配置快照、配置加载和配置校验能力。它不是 gateway 私有模块：`ConfigManager<T>` 是泛型组件，gateway 可以用它管理 `RuntimeConfig`，version app 也可以用它管理自己的业务配置。

## Boundary

| 允许 | 禁止 |
|---|---|
| 泛型配置快照管理 | 启动或停止 version app 进程 |
| 配置加载器和校验器扩展点 | 解释业务协议 |
| Gateway 运行配置 schema | Gateway 控制台指令 |
| 配置重载触发来源 | HTTP 转发实现 |

## Modules

| 模块 | 文件 | 职责 |
|---|---|---|
| `schema` | `src/schema.rs` | `RuntimeConfig`、`ServerConfig`、`VersionConfig`、`WatchConfig`、`DrainConfig` |
| `extension` | `src/extension.rs` | `ConfigLoader<T>`、`ConfigValidator<T>`、`ConfigValidatorChain<T>` |
| `manager` | `src/manager.rs` | `ConfigManager<T>` 当前快照和原子替换 |

## RuntimeConfig

| 字段 | 说明 |
|---|---|
| `revision` | 当前配置修订号 |
| `server.listen` | Gateway 对外监听地址 |
| `active_version` | 当前接收新请求的部署版本 |
| `versions` | version id 到 endpoint / health 的映射 |
| `watch` | 文件监听参数，当前 schema 已保留 |
| `drain` | 长连接排水策略 |

`RuntimeConfig` 只描述 gateway 必须理解的通用字段。业务配置必须由 version app 自己定义类型，再交给 `ConfigManager<T>` 管理。

## Snapshot Lifecycle

```text
reload or apply
  -> build next config
  -> validate next config
  -> wrap in Arc
  -> replace ArcSwap snapshot
  -> request side reads next snapshot
```

请求侧只读 `Arc<T>` 快照。控制侧通过 `reload` 或 `apply` 构造新快照，校验失败时保留旧配置。

## Extension Points

| Trait / Type | 用途 |
|---|---|
| `ConfigLoader<T>` | 根据 next revision 加载配置 |
| `ConfigValidator<T>` | 校验配置是否可应用 |
| `ConfigValidatorChain<T>` | 按顺序执行多个校验器 |
| `StaticConfigLoader<T>` | 使用内存模板返回固定配置 |
| `ConfigManager::static_config` | 给 version app 轻量业务配置使用 |

## Validation Rules

| 规则 | 当前实现 |
|---|---|
| active version 不能为空 | `DefaultConfigValidator` |
| active version 必须存在于 `versions` | `DefaultConfigValidator` |
| version id 不能为空 | `DefaultConfigValidator` |
| version endpoint 不能为空 | `DefaultConfigValidator` |
| 业务配置校验 | 调用方自定义 validator 或闭包 |

## Known Scope

| 能力 | 状态 |
|---|---|
| 泛型配置快照 | 已实现 |
| 固定配置 loader | 已实现 |
| 校验链 | 已实现 |
| Gateway 默认 schema | 已实现 |
| 文件监听 loader | 未实现 |
| TOML / YAML / JSON loader | 未实现 |

## Audit Checklist

| 项 | 要求 |
|---|---|
| 请求热路径 | 只能读取快照，不持有写锁 |
| 替换顺序 | 必须先校验后替换 |
| Gateway / version app 共用 | 泛型 API 不能被 gateway 专属逻辑污染 |
| 文件监听 | 未实现时不能在 README 或代码里假装已完成 |
