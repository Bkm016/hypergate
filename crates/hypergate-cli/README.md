# hypergate-cli

`hypergate-cli` 是 HyperGate 的本地控制台和命令树底座。它只提供注册、分发、补全和输出格式，不包含 gateway 指令，也不包含业务指令。

## Boundary

| 允许 | 禁止 |
|---|---|
| 命令注册表 | `version switch` 等 gateway 专属指令 |
| 命令树 help | `app config` 等业务指令 |
| Tab 补全接口 | HTTP 管理 API |
| 同进程控制台输入 | 文件投递控制通道 |
| 表格和面板格式 | 控制端口 |

调用方负责注册命令、注入状态和补全数据。命令树是 help、补全和执行分发的唯一真源。

## Modules

| 模块 | 文件 | 职责 |
|---|---|---|
| `command` | `src/command/` | 命令注册表、执行分发、参数策略、help 树 |
| `console` | `src/console.rs` | 同进程控制台、rustyline 补全、控制循环 |
| `format` | `src/format/` | 控制台主题、面板、表格 |
| `macros` | `src/macros.rs` | `hypergate_command!`、`hypergate_commands!` |

## Command Model

| 类型 | 说明 |
|---|---|
| `RegisteredCommand` | 一条命令声明，包含 path、usage、description、scope、completion、arguments、handler |
| `CommandRegistry` | 不可变命令注册表，负责执行和补全 |
| `CommandContext` | handler 读取注册表和调用方状态的入口 |
| `CommandOutput` | handler 返回的摘要和渲染文本 |
| `CompletionProvider` | 调用方提供动态参数补全 |

## Macros

推荐使用宏注册命令，减少样板代码。

```rust
hypergate_commands![
    app info, "show app info", |context| {
        let state = context.state::<MyState>()?;
        Ok(CommandOutput {
            summary: "app=demo".to_owned(),
            rendered: render_panel("App", vec![
                ("name".to_owned(), state.name.clone()),
            ], Vec::new()),
        })
    };
]
```

| 宏 | 用途 |
|---|---|
| `hypergate_command!` | 创建单条命令 |
| `hypergate_commands!` | 创建命令列表 |

默认参数策略是 `CommandArguments::None`。需要参数时显式设置 `arguments: Any` 和 `usage`。

## Console Behavior

| 行为 | 说明 |
|---|---|
| 输入来源 | 当前进程的 stdin |
| 补全 | 交互式控制台使用命令树和 `CompletionProvider` |
| 纯文本模式 | 设置 `HYPERGATE_PLAIN_CONSOLE=1` 后关闭 rustyline，便于测试脚本托管 stdin |
| 输出 | 命令执行完成后再打印结果和下一次 prompt |
| help | 根据命令注册表生成，不手写第二套 help |

## Audit Checklist

| 项 | 要求 |
|---|---|
| 指令归属 | gateway 和业务指令必须在调用方注册 |
| 控制通道 | 只保留运行中进程控制台 |
| 补全来源 | 不维护独立 alias 表或第二套命令树 |
| 输出格式 | 面板、表格、help 走 `format` 和 `command::help` |
| 状态注入 | handler 通过 `CommandContext::state` 读取调用方状态 |
