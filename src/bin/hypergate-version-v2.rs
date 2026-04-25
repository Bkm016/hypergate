//! Demo v2 业务进程入口。
//!
//! @author sky

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use hypergate_core::HyperResult;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("version failed: {error}");
        std::process::exit(1);
    }
}

/// 启动 v2 业务进程。
async fn run() -> HyperResult<()> {
    hypergate_version_app::run(localhost(9102)).await
}

/// 构造本机监听地址。
fn localhost(port: u16) -> SocketAddr {
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
}
