//! 默认反向代理入口。

use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use http::uri::PathAndQuery;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hypergate_core::{HyperError, HyperResult};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::HttpState;

use super::body::LimitedBodyStream;
use super::gateway::PreparedEndpoint;
use super::stream::LeaseStream;

/// 启动 HTTP 服务。监听地址取自 `state.server.listen`,避免重复传参。
pub(crate) async fn serve(state: HttpState, shutdown: CancellationToken) -> HyperResult<()> {
    let listen = state.server.listen;
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|e| HyperError::new(format!("bind failed: {e}")))?;
    serve_listener(state, listener, shutdown).await
}

/// 在已绑定 listener 上运行代理，供生产入口和真实 socket 行为测试共用。
async fn serve_listener(
    state: HttpState,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> HyperResult<()> {
    // 有界全局连接 semaphore:超出上限的新连接立即释放,不进入 serve_connection。
    // `max_connections == 0` 表示显式禁用上限,此时不限并发。
    let max_connections = state.server.max_connections;
    let semaphore = (max_connections > 0).then(|| Arc::new(Semaphore::new(max_connections)));
    // 在循环外拷贝出 header_read_timeout,避免在 service move 之后访问 state.server。
    let header_read_timeout = state.server.header_read_timeout;
    loop {
        let accepted = tokio::select! {
            _ = shutdown.cancelled() => break,
            accepted = listener.accept() => accepted,
        };
        let (stream, peer) =
            accepted.map_err(|e| HyperError::new(format!("accept failed: {e}")))?;
        // 超额连接在此处快速失败:拿不到 permit 就立即关闭 socket,不 spawn task。
        let permit = match &semaphore {
            Some(sem) => match sem.clone().try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    // 超过全局连接上限,直接丢弃新连接,避免拖累已有连接的处理。
                    drop(stream);
                    continue;
                }
            },
            None => None,
        };
        let state = state.clone();
        let tasks = state.tasks.clone();
        let connection_shutdown = shutdown.clone();
        // 每轮独立拷贝超时值,任务内仅持有值类型,不跨循环 move state/timer。
        let connection_header_read_timeout = header_read_timeout;
        tasks.spawn(async move {
            // permit 持有到连接结束,释放后才能接纳新连接。
            let _permit = permit;
            let io = TokioIo::new(stream);
            let service = service_fn(move |request: http::Request<Incoming>| {
                let state = state.clone();
                async move { Ok::<_, Infallible>(forward_incoming(state, peer, request).await) }
            });
            // 每个连接任务内新建 Timer,避免 timer 跨循环 move。
            let mut builder = http1::Builder::new();
            builder.timer(TokioTimer::new());
            // 始终显式设置:超时为 0 时传 None 明确禁用 hyper 自带的 30s 默认值。
            builder.header_read_timeout(
                (!connection_header_read_timeout.is_zero())
                    .then_some(connection_header_read_timeout),
            );
            // 单连接错误通常来自客户端提前断开,不能影响监听循环。
            let connection = builder.serve_connection(io, service).with_upgrades();
            tokio::pin!(connection);
            tokio::select! {
                _ = &mut connection => {}
                _ = connection_shutdown.cancelled() => {
                    connection.as_mut().graceful_shutdown();
                    let _ = connection.await;
                }
            }
        });
    }
    state.tasks.close();
    if tokio::time::timeout(state.server.shutdown_timeout, state.tasks.wait())
        .await
        .is_err()
    {
        eprintln!(
            "gateway connection drain timed out after {:?}",
            state.server.shutdown_timeout
        );
    }
    Ok(())
}

/// 适配 hyper 原始请求并把内部错误转换成 HTTP 响应。
async fn forward_incoming(
    state: HttpState,
    peer: SocketAddr,
    mut request: http::Request<Incoming>,
) -> Response {
    prepare_forwarded_headers(request.headers_mut(), peer, &state.server.trusted_proxies);
    let client_upgrade =
        is_websocket_upgrade(request.headers()).then(|| hyper::upgrade::on(&mut request));
    let (parts, body) = request.into_parts();
    let request = http::Request::from_parts(parts, Body::new(body));
    forward_request(state, request, client_upgrade)
        .await
        .unwrap_or_else(internal_proxy_error)
}

/// 记录内部 version 转发错误并向公网返回固定响应。
fn internal_proxy_error(error: HyperError) -> Response {
    eprintln!("gateway proxy request failed: {error}");
    (StatusCode::BAD_GATEWAY, "service temporarily unavailable\n").into_response()
}

/// 执行单次请求转发,并把响应流和版本租约绑定到同一生命周期。
async fn forward_request(
    state: HttpState,
    request: http::Request<Body>,
    client_upgrade: Option<hyper::upgrade::OnUpgrade>,
) -> HyperResult<Response> {
    let kind = state.classifier.classify(&request);
    // 默认反代只替换版本 endpoint,保留 method、path、query、body 和端到端 header。
    let mut prepared = state.gateway.prepare_proxy(kind)?;
    let endpoint_uri = build_endpoint_uri(&prepared.endpoint, request.uri())?;
    let (parts, body) = request.into_parts();
    if request_body_too_large(&parts.headers, state.body_policy.max_request_body_bytes) {
        return Ok((StatusCode::PAYLOAD_TOO_LARGE, "request body too large\n").into_response());
    }
    let should_stream = request_body_should_stream(&parts.method, &parts.headers);
    let body_limit_exceeded = Arc::new(AtomicBool::new(false));
    let websocket_request = client_upgrade.is_some();
    let connection_headers = connection_header_names(&parts.headers);
    let mut version_request = http::Request::builder()
        .method(parts.method.clone())
        .uri(endpoint_uri);

    for (name, value) in &parts.headers {
        if request_header_blocked(name, &connection_headers, websocket_request) {
            continue;
        }
        version_request = version_request.header(name, value);
    }

    let version_body = if should_stream {
        // 请求体按流转发并执行总量限制,避免高并发下把大 body 聚合进内存。
        let body = LimitedBodyStream {
            inner: Box::pin(body.into_data_stream()),
            policy: state.body_policy,
            forwarded: 0,
            exceeded: body_limit_exceeded.clone(),
        };
        Body::from_stream(body)
    } else {
        Body::empty()
    };
    let version_request = version_request
        .body(version_body)
        .map_err(|e| HyperError::new(format!("build version request failed: {e}")))?;
    // 响应头等待只在“拿到响应头之前”施加 deadline,拿到响应头后立即取消定时,
    // 这样响应 body / SSE 流不受总时长限制,避免切断长响应。
    let response_header_timeout = state.server.version_response_header_timeout;
    let request_future = state.clients.select(kind).request(version_request);
    let response = async {
        if response_header_timeout.is_zero() {
            request_future
                .await
                .map_err(|error| HyperError::new(format!("version request failed: {error}")))
        } else {
            tokio::time::timeout(response_header_timeout, request_future)
                .await
                .map_err(|_| {
                    HyperError::new(format!(
                        "version response header timeout after {response_header_timeout:?}"
                    ))
                })?
                .map_err(|error| HyperError::new(format!("version request failed: {error}")))
        }
    };
    let cancel = prepared.lease.cancel_token();
    let response_result = tokio::select! {
        result = response => result,
        _ = cancel.cancelled() => {
            return Err(HyperError::new("version request cancelled while draining"));
        }
    };
    let mut version_response = match response_result {
        Ok(_) if body_limit_exceeded.load(Ordering::Acquire) => {
            return Ok((StatusCode::PAYLOAD_TOO_LARGE, "request body too large\n").into_response());
        }
        Ok(response) => response,
        Err(_) if body_limit_exceeded.load(Ordering::Acquire) => {
            return Ok((StatusCode::PAYLOAD_TOO_LARGE, "request body too large\n").into_response());
        }
        Err(error) => return Err(error),
    };
    let websocket_response =
        websocket_request && version_response.status() == StatusCode::SWITCHING_PROTOCOLS;
    let upstream_upgrade = websocket_response.then(|| hyper::upgrade::on(&mut version_response));
    let (response_parts, response_body) = version_response.into_parts();
    let mut response = Response::builder().status(response_parts.status);
    let response_connection_headers = connection_header_names(&response_parts.headers);
    for (name, value) in &response_parts.headers {
        if response_header_blocked(name, &response_connection_headers, websocket_response) {
            continue;
        }
        response = response.header(name, value);
    }

    if websocket_response {
        prepared.lease.promote_stream();
        let client_upgrade = client_upgrade
            .ok_or_else(|| HyperError::new("client websocket upgrade is unavailable"))?;
        let upstream_upgrade = upstream_upgrade
            .ok_or_else(|| HyperError::new("version websocket upgrade is unavailable"))?;
        state.tasks.spawn(async move {
            let lease = prepared.lease;
            let cancel = lease.cancel_token();
            if let Err(error) = relay_upgrades(client_upgrade, upstream_upgrade, cancel).await {
                eprintln!("gateway websocket tunnel failed: {error}");
            }
            drop(lease);
        });
        return response
            .body(Body::empty())
            .map_err(|error| HyperError::new(format!("build websocket response failed: {error}")));
    }

    if is_event_stream(&response_parts.headers) {
        prepared.lease.promote_stream();
    }

    // 响应体流持有版本租约,流结束或 version app 请求错误时自动释放连接计数。
    let stream = LeaseStream::new(Body::new(response_body).into_data_stream(), prepared.lease);
    let body = Body::from_stream(stream);

    response
        .body(body)
        .map_err(|e| HyperError::new(format!("build response failed: {e}")))
}

/// 用 active version endpoint 替换请求目标主机,保留原始 path 和 query。
fn build_endpoint_uri(endpoint: &PreparedEndpoint, uri: &http::Uri) -> HyperResult<http::Uri> {
    let path_and_query = if endpoint.base_path.is_empty() {
        uri.path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/"))
    } else {
        let path_and_query = uri
            .path_and_query()
            .map(|value| value.as_str())
            .unwrap_or("/");
        format!("{}{}", endpoint.base_path, path_and_query)
            .parse()
            .map_err(|e| HyperError::new(format!("invalid version endpoint: {e}")))?
    };
    http::Uri::builder()
        .scheme(endpoint.scheme.clone())
        .authority(endpoint.authority.clone())
        .path_and_query(path_and_query)
        .build()
        .map_err(|e| HyperError::new(format!("invalid version endpoint: {e}")))
}

/// 通过 `Content-Length` 提前拒绝明显超过上限的请求体。
fn request_body_too_large(headers: &HeaderMap, limit: usize) -> bool {
    headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > limit)
}

/// 判断请求 Header 是否应由 Gateway 重建或丢弃。
fn request_header_blocked(
    name: &HeaderName,
    connection_headers: &HashSet<HeaderName>,
    preserve_upgrade: bool,
) -> bool {
    name == http::header::HOST || hop_by_hop_header(name, connection_headers, preserve_upgrade)
}

/// 判断响应 Header 是否只属于当前 TCP hop。
fn response_header_blocked(
    name: &HeaderName,
    connection_headers: &HashSet<HeaderName>,
    preserve_upgrade: bool,
) -> bool {
    hop_by_hop_header(name, connection_headers, preserve_upgrade)
}

/// 过滤标准逐跳 Header 和 `Connection` 额外声明的 Header。
fn hop_by_hop_header(
    name: &HeaderName,
    connection_headers: &HashSet<HeaderName>,
    preserve_upgrade: bool,
) -> bool {
    if preserve_upgrade && (name == http::header::CONNECTION || name == http::header::UPGRADE) {
        return false;
    }
    if name == http::header::CONNECTION || connection_headers.contains(name) {
        return true;
    }
    if name == http::header::PROXY_AUTHENTICATE {
        return true;
    }
    if name == http::header::PROXY_AUTHORIZATION {
        return true;
    }
    if name == http::header::TE {
        return true;
    }
    if name == http::header::TRAILER {
        return true;
    }
    if name == http::header::TRANSFER_ENCODING {
        return true;
    }
    if name == http::header::UPGRADE {
        return true;
    }
    name.as_str().eq_ignore_ascii_case("keep-alive")
        || name.as_str().eq_ignore_ascii_case("proxy-connection")
}

/// 解析 `Connection` 中额外声明的逐跳 Header 名称。
fn connection_header_names(headers: &HeaderMap) -> HashSet<HeaderName> {
    headers
        .get_all(http::header::CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|value| HeaderName::from_bytes(value.trim().as_bytes()).ok())
        .collect()
}

/// 仅信任明确配置的直接上游代理提供的客户端转发 Header。
fn prepare_forwarded_headers(
    headers: &mut HeaderMap,
    peer: SocketAddr,
    trusted_proxies: &[ipnet::IpNet],
) {
    let trusted = trusted_proxies
        .iter()
        .any(|network| network.contains(&peer.ip()));
    let original_host = headers.get(http::header::HOST).cloned();
    let forwarded_for = HeaderName::from_static("x-forwarded-for");
    let forwarded_proto = HeaderName::from_static("x-forwarded-proto");
    let forwarded_host = HeaderName::from_static("x-forwarded-host");
    if !trusted {
        headers.remove(http::header::FORWARDED);
        headers.remove(&forwarded_for);
        headers.remove(&forwarded_proto);
        headers.remove(&forwarded_host);
        headers.remove(HeaderName::from_static("x-real-ip"));
        headers.remove(HeaderName::from_static("cf-connecting-ip"));
    }
    if !headers.contains_key(&forwarded_for)
        && let Ok(value) = HeaderValue::from_str(&peer.ip().to_string())
    {
        headers.insert(forwarded_for, value);
    }
    if !headers.contains_key(&forwarded_proto) {
        headers.insert(forwarded_proto, HeaderValue::from_static("http"));
    }
    if !headers.contains_key(&forwarded_host)
        && let Some(host) = original_host
    {
        headers.insert(forwarded_host, host);
    }
}

/// 判断请求是否要求建立 WebSocket 隧道。
fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::UPGRADE)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
}

/// 判断 version app 响应是否为 SSE。
fn is_event_stream(headers: &HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
}

/// 等待两侧升级成功并转发 WebSocket 双向字节流。
async fn relay_upgrades(
    client: hyper::upgrade::OnUpgrade,
    upstream: hyper::upgrade::OnUpgrade,
    cancel: tokio_util::sync::CancellationToken,
) -> HyperResult<()> {
    let client = client
        .await
        .map_err(|error| HyperError::new(format!("client upgrade failed: {error}")))?;
    let upstream = upstream
        .await
        .map_err(|error| HyperError::new(format!("version upgrade failed: {error}")))?;
    let mut client = TokioIo::new(client);
    let mut upstream = TokioIo::new(upstream);
    tokio::select! {
        result = tokio::io::copy_bidirectional(&mut client, &mut upstream) => {
            result.map_err(|error| HyperError::new(format!("websocket copy failed: {error}")))?;
        }
        _ = cancel.cancelled() => {}
    }
    Ok(())
}

/// 判断请求体是否需要作为流转发给 version app。
fn request_body_should_stream(method: &Method, headers: &HeaderMap) -> bool {
    if headers.get(http::header::TRANSFER_ENCODING).is_some() {
        return true;
    }
    if headers
        .get(http::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > 0)
    {
        return true;
    }
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{
        connection_header_names, internal_proxy_error, prepare_forwarded_headers,
        response_header_blocked, serve_listener,
    };
    use crate::http::{
        DefaultRequestKindClassifier, Gateway, HttpState, ProxyBodyPolicy, VersionClients,
    };
    use crate::runtime::VersionRegistry;
    use axum::Router;
    use axum::body::{Body, to_bytes};
    use axum::extract::ws::{Message as AxumMessage, WebSocketUpgrade};
    use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
    use axum::response::Response;
    use axum::routing::{get, post};
    use futures_util::{SinkExt, StreamExt};
    use hypergate_config::{RuntimeConfig, VersionConfig};
    use hypergate_core::VersionId;
    use hypergate_core::{HyperError, HyperResult};
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;
    use tokio_tungstenite::tungstenite::Message as ClientMessage;
    use tokio_util::sync::CancellationToken;
    use tokio_util::task::TaskTracker;

    /// 公网 502 不得包含 version app、地址、超时或底层网络诊断。
    #[tokio::test]
    async fn internal_proxy_error_hides_diagnostics() {
        let response = internal_proxy_error(HyperError::new(
            "version request failed: tcp connect to 127.0.0.1:9102 timed out after 30s",
        ));
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = to_bytes(response.into_body(), 1024)
            .await
            .expect("stable gateway error body");
        assert_eq!(body.as_ref(), b"service temporarily unavailable\n");
    }

    /// 端到端响应 Header 必须保留，逐跳 Header 及 Connection 扩展必须剔除。
    #[test]
    fn response_headers_preserve_end_to_end_semantics() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("keep-alive, x-internal"),
        );
        let connection_headers = connection_header_names(&headers);
        assert!(!response_header_blocked(
            &http::header::SET_COOKIE,
            &connection_headers,
            false,
        ));
        assert!(!response_header_blocked(
            &http::header::LOCATION,
            &connection_headers,
            false,
        ));
        assert!(!response_header_blocked(
            &http::header::CONTENT_LENGTH,
            &connection_headers,
            false,
        ));
        assert!(response_header_blocked(
            &http::header::CONNECTION,
            &connection_headers,
            false,
        ));
        assert!(response_header_blocked(
            &HeaderName::from_static("x-internal"),
            &connection_headers,
            false,
        ));
    }

    /// 非可信来源不能伪造客户端 IP，可信本机代理则保留已清洗的转发链。
    #[test]
    fn forwarded_headers_respect_trusted_proxy_boundary() {
        let mut direct = HeaderMap::new();
        direct.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.10"));
        direct.insert("cf-connecting-ip", HeaderValue::from_static("203.0.113.10"));
        prepare_forwarded_headers(
            &mut direct,
            "198.51.100.8:4000".parse().expect("peer address"),
            &[],
        );
        assert_eq!(direct["x-forwarded-for"], "198.51.100.8");
        assert!(!direct.contains_key("cf-connecting-ip"));

        let mut proxied = HeaderMap::new();
        proxied.insert("x-forwarded-for", HeaderValue::from_static("203.0.113.10"));
        prepare_forwarded_headers(
            &mut proxied,
            "127.0.0.1:4000".parse().expect("proxy address"),
            &["127.0.0.0/8".parse().expect("trusted network")],
        );
        assert_eq!(proxied["x-forwarded-for"], "203.0.113.10");
    }

    /// 真实代理链必须保留重复 Cookie、重定向和业务自定义 Header。
    #[tokio::test]
    async fn proxy_preserves_application_response_headers() {
        let stack = TestStack::start().await;
        let client = VersionClients::new(Duration::from_secs(1));
        let request = http::Request::get(format!("http://{}/headers", stack.gateway_addr))
            .body(Body::empty())
            .expect("request should build");
        let response = client
            .unary
            .request(request)
            .await
            .expect("gateway request should succeed");
        assert_eq!(
            response
                .headers()
                .get_all(http::header::SET_COOKIE)
                .iter()
                .count(),
            2
        );
        assert_eq!(response.headers()[http::header::LOCATION], "/next");
        assert_eq!(response.headers()["x-hypergate-test"], "forwarded");
        stack.stop().await;
    }

    /// WebSocket upgrade 必须穿透 Gateway，并把隧道生命周期计入版本租约。
    #[tokio::test]
    async fn websocket_tunnel_forwards_frames() {
        let stack = TestStack::start().await;
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://{}/ws", stack.gateway_addr))
                .await
                .expect("websocket should connect through gateway");
        socket
            .send(ClientMessage::Text("hello".into()))
            .await
            .expect("websocket message should send");
        let message = socket
            .next()
            .await
            .expect("websocket should return a frame")
            .expect("websocket frame should decode");
        assert_eq!(message, ClientMessage::Text("hello".into()));
        socket.close(None).await.expect("websocket should close");
        stack.stop().await;
    }

    /// Gateway 停止监听后必须继续服务已经升级的 WebSocket，直到客户端关闭。
    #[tokio::test]
    async fn websocket_drains_after_gateway_shutdown() {
        let stack = TestStack::start().await;
        let (mut socket, _) =
            tokio_tungstenite::connect_async(format!("ws://{}/ws", stack.gateway_addr))
                .await
                .expect("websocket should connect through gateway");
        stack.gateway_shutdown.cancel();
        socket
            .send(ClientMessage::Text("during-drain".into()))
            .await
            .expect("draining websocket should remain writable");
        let message = socket
            .next()
            .await
            .expect("draining websocket should return a frame")
            .expect("draining websocket frame should decode");
        assert_eq!(message, ClientMessage::Text("during-drain".into()));
        socket.close(None).await.expect("websocket should close");
        stack.stop().await;
    }

    /// Chunked 请求体超过限制后必须返回 413，不能伪装成上游 502。
    #[tokio::test]
    async fn chunked_body_limit_returns_payload_too_large() {
        let stack = TestStack::start_with_limit(4).await;
        let client = VersionClients::new(Duration::from_secs(1));
        let body = Body::from_stream(futures_util::stream::iter(vec![
            Ok::<_, Infallible>(bytes::Bytes::from_static(b"abc")),
            Ok::<_, Infallible>(bytes::Bytes::from_static(b"def")),
        ]));
        let request = http::Request::post(format!("http://{}/upload", stack.gateway_addr))
            .body(body)
            .expect("request should build");
        let response = client
            .unary
            .request(request)
            .await
            .expect("gateway should return an HTTP response");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        stack.stop().await;
    }

    struct TestStack {
        gateway_addr: SocketAddr,
        gateway_shutdown: CancellationToken,
        upstream_shutdown: CancellationToken,
        gateway: JoinHandle<HyperResult<()>>,
        upstream: JoinHandle<Result<(), std::io::Error>>,
    }

    impl TestStack {
        async fn start() -> Self {
            Self::start_with_limit(ProxyBodyPolicy::DEFAULT_MAX_REQUEST_BODY_BYTES).await
        }

        async fn start_with_limit(max_request_body_bytes: usize) -> Self {
            let upstream_listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("upstream should bind");
            let upstream_addr = upstream_listener.local_addr().expect("upstream address");
            let upstream_shutdown = CancellationToken::new();
            let upstream_signal = upstream_shutdown.clone();
            let upstream = tokio::spawn(async move {
                axum::serve(
                    upstream_listener,
                    Router::new()
                        .route("/headers", get(header_response))
                        .route("/ws", get(websocket_echo))
                        .route("/upload", post(upload_body))
                        .route("/health", get(|| async { StatusCode::NO_CONTENT })),
                )
                .with_graceful_shutdown(upstream_signal.cancelled_owned())
                .await
            });

            let mut config = RuntimeConfig::minimal();
            config.server.shutdown_timeout = Duration::from_secs(2);
            config.active_version = VersionId::new("test");
            config.versions.insert(
                VersionId::new("test"),
                VersionConfig {
                    endpoint: format!("http://{upstream_addr}"),
                    health: format!("http://{upstream_addr}/health"),
                },
            );
            let versions = Arc::new(VersionRegistry::new());
            versions
                .ensure(config.active_version.clone())
                .expect("version should register")
                .activate();
            let gateway =
                Arc::new(Gateway::new(&config, versions).expect("gateway core should initialize"));
            let gateway_listener = TcpListener::bind("127.0.0.1:0")
                .await
                .expect("gateway should bind");
            let gateway_addr = gateway_listener.local_addr().expect("gateway address");
            let gateway_shutdown = CancellationToken::new();
            let gateway_signal = gateway_shutdown.clone();
            let state = HttpState {
                gateway,
                clients: VersionClients::new(Duration::from_secs(1)),
                classifier: Arc::new(DefaultRequestKindClassifier),
                body_policy: ProxyBodyPolicy {
                    max_request_body_bytes,
                },
                server: config.server,
                tasks: TaskTracker::new(),
            };
            let gateway = tokio::spawn(serve_listener(state, gateway_listener, gateway_signal));
            Self {
                gateway_addr,
                gateway_shutdown,
                upstream_shutdown,
                gateway,
                upstream,
            }
        }

        async fn stop(self) {
            self.gateway_shutdown.cancel();
            self.upstream_shutdown.cancel();
            self.gateway
                .await
                .expect("gateway task should join")
                .expect("gateway should stop cleanly");
            self.upstream
                .await
                .expect("upstream task should join")
                .expect("upstream should stop cleanly");
        }
    }

    async fn header_response() -> Response {
        let mut response = Response::new(Body::from("ok"));
        response.headers_mut().append(
            http::header::SET_COOKIE,
            HeaderValue::from_static("a=1; Path=/"),
        );
        response.headers_mut().append(
            http::header::SET_COOKIE,
            HeaderValue::from_static("b=2; Path=/"),
        );
        response
            .headers_mut()
            .insert(http::header::LOCATION, HeaderValue::from_static("/next"));
        response.headers_mut().insert(
            HeaderName::from_static("x-hypergate-test"),
            HeaderValue::from_static("forwarded"),
        );
        response
    }

    async fn websocket_echo(upgrade: WebSocketUpgrade) -> Response {
        upgrade.on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.next().await {
                if matches!(message, AxumMessage::Text(_) | AxumMessage::Binary(_))
                    && socket.send(message).await.is_err()
                {
                    break;
                }
            }
        })
    }

    async fn upload_body(request: axum::extract::Request) -> StatusCode {
        match to_bytes(request.into_body(), 1024).await {
            Ok(_) => StatusCode::NO_CONTENT,
            Err(_) => StatusCode::BAD_REQUEST,
        }
    }
}
