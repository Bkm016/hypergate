//! CLI 视图格式化。

use std::net::SocketAddr;

use crate::runtime::VersionRegistry;
use hypergate_config::{ConfigValidator, DefaultConfigValidator, RuntimeConfig};
use hypergate_core::{HyperResult, VersionId, VersionState};

use hypergate_cli::format::{Align, Table, column, render_panel};

/// 渲染 gateway 当前状态总览。
pub(crate) fn format_status(
    config: &RuntimeConfig,
    versions: &VersionRegistry,
) -> HyperResult<String> {
    let summary = version_summary(versions)?;
    Ok(render_panel(
        "Gateway Status",
        Vec::new(),
        vec![Table {
            title: "Summary".to_owned(),
            columns: vec![
                column("active", Align::Left),
                column("revision", Align::Right),
                column("versions", Align::Right),
                column("draining", Align::Right),
                column("stopped", Align::Right),
                column("requests", Align::Right),
                column("streams", Align::Right),
            ],
            rows: vec![vec![
                config.active_version.value.to_string(),
                config.revision.value.to_string(),
                summary.total.to_string(),
                summary.draining.to_string(),
                summary.stopped.to_string(),
                summary.active_requests.to_string(),
                summary.active_streams.to_string(),
            ]],
        }],
    ))
}

/// 渲染当前运行配置。
pub(crate) fn format_config(config: &RuntimeConfig) -> String {
    render_panel(
        "Config",
        vec![
            ("listen".to_owned(), config.server.listen.to_string()),
            ("active".to_owned(), config.active_version.value.to_string()),
            ("revision".to_owned(), config.revision.value.to_string()),
            (
                "drain_timeout".to_owned(),
                format!("{}s", config.drain.timeout.as_secs()),
            ),
            (
                "force_close".to_owned(),
                config.drain.force_close_streams.to_string(),
            ),
        ],
        vec![version_endpoints_table(config)],
    )
}

/// 校验当前配置并渲染结果。
pub(crate) fn format_check_result(config: &RuntimeConfig) -> HyperResult<String> {
    let validator = DefaultConfigValidator;
    validator.validate(config)?;
    Ok(render_panel(
        "Config Check",
        vec![("result".to_owned(), "ok".to_owned())],
        Vec::new(),
    ))
}

/// 渲染版本 endpoint、运行状态和连接计数。
pub(crate) fn versions_table(
    config: &RuntimeConfig,
    versions: &VersionRegistry,
) -> HyperResult<Table> {
    let rows = versions
        .snapshots()?
        .into_iter()
        .map(|snapshot| {
            let version = config
                .versions
                .get(&VersionId::new(snapshot.id.value.clone()));
            let drain_elapsed = snapshot
                .drain_elapsed_secs
                .map(|value| {
                    if value >= config.drain.timeout.as_secs() {
                        format!("{value}s timeout")
                    } else {
                        format!("{value}s")
                    }
                })
                .unwrap_or_else(|| "-".to_owned());
            vec![
                snapshot.id.value.to_string(),
                snapshot.state.as_str().to_owned(),
                version
                    .map(|version| version.endpoint.clone())
                    .unwrap_or_else(|| "-".to_owned()),
                version
                    .map(|version| version.health.clone())
                    .unwrap_or_else(|| "-".to_owned()),
                snapshot.active_requests.to_string(),
                snapshot.active_streams.to_string(),
                snapshot.total_requests.to_string(),
                drain_elapsed,
            ]
        })
        .collect();
    Ok(Table {
        title: "Versions".to_owned(),
        columns: vec![
            column("version", Align::Left),
            column("state", Align::Left),
            column("endpoint", Align::Left),
            column("health", Align::Left),
            column("requests", Align::Right),
            column("streams", Align::Right),
            column("total", Align::Right),
            column("drain", Align::Right),
        ],
        rows,
    })
}

/// Gateway 状态页使用的版本聚合指标。
struct VersionSummary {
    /// 已注册版本数量。
    total: usize,
    /// draining 状态版本数量。
    draining: usize,
    /// stopped 状态版本数量。
    stopped: usize,
    /// 当前活跃请求总数。
    active_requests: u64,
    /// 当前活跃长连接总数。
    active_streams: u64,
}

/// 从版本快照聚合状态页摘要。
fn version_summary(versions: &VersionRegistry) -> HyperResult<VersionSummary> {
    let snapshots = versions.snapshots()?;
    let mut summary = VersionSummary {
        total: snapshots.len(),
        draining: 0,
        stopped: 0,
        active_requests: 0,
        active_streams: 0,
    };
    for snapshot in snapshots {
        if snapshot.state == VersionState::Draining {
            summary.draining += 1;
        }
        if snapshot.state == VersionState::Stopped {
            summary.stopped += 1;
        }
        summary.active_requests += snapshot.active_requests;
        summary.active_streams += snapshot.active_streams;
    }
    Ok(summary)
}

/// 渲染配置中的 version app endpoint 清单。
pub(crate) fn version_endpoints_table(config: &RuntimeConfig) -> Table {
    let mut rows: Vec<_> = config
        .versions
        .iter()
        .map(|(id, version)| {
            vec![
                id.value.to_string(),
                version.endpoint.clone(),
                version.health.clone(),
            ]
        })
        .collect();
    rows.sort_by(|a, b| a[0].cmp(&b[0]));
    Table {
        title: "Versions".to_owned(),
        columns: vec![
            column("version", Align::Left),
            column("endpoint", Align::Left),
            column("health", Align::Left),
        ],
        rows,
    }
}

/// 渲染进程启动 banner。
pub(crate) fn start_output(listen: SocketAddr, config: &RuntimeConfig) -> String {
    format!(
        "HyperGate started  listen=http://{listen}  active={}  versions={}",
        config.active_version.value,
        config.versions.len(),
    )
}
