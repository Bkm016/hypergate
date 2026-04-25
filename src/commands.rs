//! Demo version app 控制台指令。
//!
//! 这里展示业务如何通过 `VersionAppConsole::builder` 追加自己的指令。
//! SDK 默认指令、help 树和补全入口仍由框架维护。
//!
//! @author sky

use hypergate_app::{
    CommandOutput, VersionAppCommandState, VersionAppConsole, VersionAppOptions,
    hypergate_commands, render_panel,
};
use hypergate_config::ReloadTrigger;

use crate::config::{DemoConfigHandle, demo_config_panel};

/// Demo 命令状态。
type DemoCommandState = VersionAppCommandState<DemoConfigHandle>;

/// 构建 demo 控制台。
pub(crate) fn demo_console(
    options: &VersionAppOptions,
    config: DemoConfigHandle,
) -> VersionAppConsole {
    VersionAppConsole::builder(options)
        .state(config)
        .commands(hypergate_commands![
            app info, "show demo app info", |context| {
                let state = context.state::<DemoCommandState>()?;
                let config = state.app.snapshot();
                Ok(CommandOutput {
                    summary: format!("app={}", config.name),
                    rendered: render_panel(
                        "Demo App",
                        vec![
                            ("name".to_owned(), config.name.clone()),
                            ("app".to_owned(), state.options.name.to_string()),
                            (
                                "listen".to_owned(),
                                format!("http://{}", state.options.listen),
                            ),
                        ],
                        Vec::new(),
                    ),
                })
            };
            app config, "show demo config", |context| {
                let state = context.state::<DemoCommandState>()?;
                let config = state.app.snapshot();
                let revision = state.app.revision().value;
                Ok(CommandOutput {
                    summary: format!("config_revision={revision}"),
                    rendered: demo_config_panel(revision, config.as_ref()),
                })
            };
            app reload, "reload demo config", |context| {
                let state = context.state::<DemoCommandState>()?;
                let config = state.app.reload(ReloadTrigger::Command {
                    source: "demo console".to_owned(),
                })?;
                let revision = state.app.revision().value;
                Ok(CommandOutput {
                    summary: format!("config_revision={revision}"),
                    rendered: demo_config_panel(revision, config.as_ref()),
                })
            };
        ])
        .build()
}
