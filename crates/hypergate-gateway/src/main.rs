//! HyperGate gateway 可执行入口。

mod app;
mod commands;
mod control;
mod default_config;
mod http;
mod management;
mod runtime;
mod views;

#[tokio::main]
async fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if let Err(error) = app::run(args, default_config::default_config()).await {
        eprintln!("hypergate failed: {error}");
        std::process::exit(1);
    }
}
