# hypergate-core

`hypergate-core` 是 HyperGate 的最底层基础 crate。它只放多个 crate 都需要共享的稳定概念，不包含运行时、不依赖配置、不依赖控制台，也不携带业务语义。

## Boundary

| 允许 | 禁止 |
|---|---|
| 错误类型 | HTTP 转发实现 |
| 配置修订号 | Gateway 指令 |
| 请求连接类型 | Version app 启动逻辑 |
| 版本标识和版本状态 | 配置加载器 |
| 扩展描述和扩展注册表 | 业务配置或业务协议 |

## Modules

| 模块 | 文件 | 职责 |
|---|---|---|
| `error` | `src/error.rs` | `HyperError`、`HyperResult` |
| `extension` | `src/extension.rs` | `ExtensionDescriptor`、`DescribedExtension`、`ExtensionRegistry` |
| `request` | `src/request.rs` | `ConfigRevision`、`RequestKind` |
| `version` | `src/version.rs` | `VersionId`、`VersionState` |

## Public API

| API | 说明 | 稳定性要求 |
|---|---|---|
| `HyperError` | 框架统一错误包装 | 当前只保留 message，避免过早固定错误分类 |
| `HyperResult<T>` | 框架统一返回类型 | 所有 crate 共享 |
| `ConfigRevision` | 配置快照修订号 | 单调递增，不复用旧 revision |
| `RequestKind` | 请求连接类型 | 只区分 `Unary` 和 `Stream` |
| `VersionId` | Gateway 配置中的部署版本标识 | 不等同于业务进程名 |
| `VersionState` | Version 运行状态 | `Active` 是唯一接收新请求的状态 |
| `ExtensionDescriptor` | 扩展元信息 | id 必须稳定 |
| `ExtensionRegistry<T>` | 通用扩展注册容器 | 重复 id 必须报错 |

## Extension Contract

扩展系统只解决注册和发现，不负责生命周期、动态加载或热更新。

| 决策 | 原因 |
|---|---|
| `ExtensionDescriptor::new` 是 `const fn` | 降低声明扩展元信息的样板代码 |
| `DescribedExtension` 只要求 `descriptor` | 不把启动、关闭、重载强塞给所有扩展 |
| `ExtensionRegistry::register` 拒绝重复 id | 避免后注册扩展静默覆盖生产行为 |
| `hypergate_extension_registry!` 返回注册表 | 让调用方可以用宏收敛静态扩展列表 |

## Audit Checklist

| 项 | 要求 |
|---|---|
| 依赖方向 | 不依赖其他 HyperGate crate |
| 语义边界 | 不出现业务、gateway 专属或 version app 启动逻辑 |
| 文档 | 公开 API 必须有 rustdoc |
| 状态枚举 | 只承诺已经被框架使用的状态和请求类型 |
