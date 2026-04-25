//! 默认反向代理入口。

use std::convert::Infallible;
use std::net::SocketAddr;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use http::uri::PathAndQuery;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use hypergate_core::{HyperError, HyperResult};
use tokio::net::TcpListener;

use super::HttpState;

use super::body::LimitedBodyStream;
use super::gateway::PreparedEndpoint;
use super::stream::LeaseStream;

/// 启动 HTTP 服务。
pub(crate) async fn serve(state: HttpState, listen: SocketAddr) -> HyperResult<()> {
    let listener = TcpListener::bind(listen)
        .await
        .map_err(|e| HyperError::new(format!("bind failed: {e}")))?;
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| HyperError::new(format!("accept failed: {e}")))?;
        let state = state.clone();
        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let service = service_fn(move |request: http::Request<Incoming>| {
                let state = state.clone();
                async move { Ok::<_, Infallible>(forward_incoming(state, request).await) }
            });
            // 单连接错误通常来自客户端提前断开,不能影响监听循环。
            let _ = http1::Builder::new().serve_connection(io, service).await;
        });
    }
}

/// 适配 hyper 原始请求并把内部错误转换成 HTTP 响应。
async fn forward_incoming(state: HttpState, request: http::Request<Incoming>) -> Response {
    let (parts, body) = request.into_parts();
    let request = http::Request::from_parts(parts, Body::new(body));
    forward_request(state, request)
        .await
        .unwrap_or_else(|error| (StatusCode::BAD_GATEWAY, format!("{error}\n")).into_response())
}

/// 执行单次请求转发,并把响应流和版本租约绑定到同一生命周期。
async fn forward_request(state: HttpState, request: http::Request<Body>) -> HyperResult<Response> {
    let kind = state.classifier.classify(&request);
    // 默认反代只替换版本 endpoint,保留 method、path、query、body 和端到端 header。
    let prepared = state.gateway.prepare_proxy(kind)?;
    let endpoint_uri = build_endpoint_uri(&prepared.endpoint, request.uri())?;
    let (parts, body) = request.into_parts();
    if request_body_too_large(&parts.headers, state.body_policy.max_request_body_bytes) {
        return Ok((StatusCode::PAYLOAD_TOO_LARGE, "request body too large\n").into_response());
    }
    let should_stream = request_body_should_stream(&parts.method, &parts.headers);
    let mut version_request = http::Request::builder()
        .method(parts.method.clone())
        .uri(endpoint_uri);

    for (name, value) in &parts.headers {
        if request_header_blocked(name) {
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
        };
        Body::from_stream(body)
    } else {
        Body::empty()
    };
    let version_request = version_request
        .body(version_body)
        .map_err(|e| HyperError::new(format!("build version request failed: {e}")))?;
    let version_response = state
        .clients
        .select(kind)
        .request(version_request)
        .await
        .map_err(|e| HyperError::new(format!("version request failed: {e}")))?;
    let (response_parts, response_body) = version_response.into_parts();
    let mut response = Response::builder().status(response_parts.status);

    for (name, value) in &response_parts.headers {
        if hop_by_hop_or_framing_header(name) {
            continue;
        }
        response = response.header(name, value);
    }

    // 响应体流持有版本租约,流结束或 version app 请求错误时自动释放连接计数。
    let stream = LeaseStream {
        inner: Box::pin(Body::new(response_body).into_data_stream()),
        lease: Some(prepared.lease),
    };
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

/// 判断请求 header 是否应由 gateway 重建或丢弃。
fn request_header_blocked(name: &HeaderName) -> bool {
    name == http::header::HOST || hop_by_hop_or_framing_header(name)
}

/// 过滤逐跳 header 和由 hyper 自动维护的 framing header。
fn hop_by_hop_or_framing_header(name: &HeaderName) -> bool {
    if name == http::header::CONNECTION {
        return true;
    }
    if name == http::header::CONTENT_LENGTH {
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
