//! HyperGate gateway 可执行入口。

/// Linux 长请求高并发使用可主动归还空闲页的 allocator，避免 glibc arena
/// 在流量峰值后长期保留进程 RSS。
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod app;
mod commands;
mod control;
mod http;
mod management;
mod options;
mod runtime;
mod state;
mod views;

#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Err(error) = app::run(args).await {
        eprintln!("hypergate failed: {error}");
        std::process::exit(1);
    }
}
