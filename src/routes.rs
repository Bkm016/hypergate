//! Demo version app HTTP 路由。
//!
//! Gateway 只切换 version endpoint,不会解释业务 path。这里的路由完全
//! 属于 version app,开发者可以替换成自己的 HTTP 能力。
//!
//! @author sky

use std::time::Duration;

use futures_util::stream;
use hypergate_app::axum::body::{Body, Bytes};
use hypergate_app::axum::response::Response;
use hypergate_app::axum::{Router, routing::get};

use crate::config::DemoConfigHandle;

/// 构建 demo HTTP 路由。
pub(crate) fn demo_router(app_name: String, config: DemoConfigHandle) -> Router {
    let stream_name = app_name.clone();
    Router::new()
        .route("/health", get(|| async { "ok\n" }))
        .route(
            "/stream",
            get(move || {
                let app_name = stream_name.clone();
                async move { stream_response(app_name) }
            }),
        )
        .fallback(move || {
            let app_name = app_name.clone();
            let config = config.clone();
            async move {
                let snapshot = config.snapshot();
                format!("{} from {}\n", snapshot.greeting, app_name)
            }
        })
}

/// 示例 SSE 响应。
///
/// 该路由用于验证 gateway 切换版本时的长连接兼容性: 已经建立的流式
/// 响应继续持有旧 version 租约,新请求才进入新的 active version。
fn stream_response(app_name: String) -> Response {
    let chunks = stream::unfold(0, move |index| {
        let app_name = app_name.clone();
        async move {
            if index >= 5 {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let line = format!("{app_name}:{index}\n");
            Some((
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(line)),
                index + 1,
            ))
        }
    });
    Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(chunks))
        .unwrap_or_else(|_| Response::new(Body::from("stream response build failed\n")))
}
