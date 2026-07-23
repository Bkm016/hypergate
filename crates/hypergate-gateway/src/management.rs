//! 本机管理 HTTP 服务。
//!
//! 管理 API 默认关闭。仅当环境变量 `HYPERGATE_ADMIN_TOKEN` 设置后启用,
//! 并从 `HYPERGATE_ADMIN_LISTEN` 读取监听地址(默认 `127.0.0.1:8090`)。
//! 监听地址必须是 loopback 地址,非 loopback 地址拒绝启动。
//!
//! 路由:
//! - `GET /api/v1/status`: 返回当前 gateway 状态快照。
//! - `POST /api/v1/actions/switch`: 切换 active version。
//! - `POST /api/v1/actions/drain`: 让非 active 版本进入 draining。
//! - `POST /api/v1/actions/stop`: 在连接清空后停止非 active 版本。
//! - `POST /api/v1/actions/rollback`: 切回上一个 active version。
//! - `POST /api/v1/actions/reload`: 重读并应用 TOML 配置。
//!
//! 所有路由要求 `Authorization: Bearer <token>` 鉴权,使用常时比较。
//! 不配置 CORS。错误响应为稳定 JSON,缺失/非法鉴权返回 401,
//! 控制冲突(如乐观锁不匹配)返回 409,非法请求返回 400,
//! 内部控制或快照错误返回 500。

#![deny(missing_docs)]

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hypergate_core::{HyperError, HyperResult, VersionId};
use tokio_util::sync::CancellationToken;

use crate::control::{ControlError, ControlOutcome, GatewayControl};

/// 管理 API 默认监听地址。
const DEFAULT_ADMIN_LISTEN: &str = "127.0.0.1:8090";

/// 管理 API 鉴权 token 环境变量名。
const ADMIN_TOKEN_ENV: &str = "HYPERGATE_ADMIN_TOKEN";

/// 管理 API 监听地址环境变量名。
const ADMIN_LISTEN_ENV: &str = "HYPERGATE_ADMIN_LISTEN";

/// 管理 token 最大字节数,固定长度比较避免暴露实际长度。
const MAX_ADMIN_TOKEN_BYTES: usize = 1024;

/// 固定长度管理凭据。
pub(crate) struct AdminCredential {
    /// 补零后的 token 字节。
    bytes: [u8; MAX_ADMIN_TOKEN_BYTES],
    /// token 实际字节数。
    length: usize,
}

impl AdminCredential {
    /// 从非空 token 构造固定长度凭据。
    fn new(token: &str) -> HyperResult<Self> {
        if token.len() > MAX_ADMIN_TOKEN_BYTES {
            return Err(HyperError::new(format!(
                "{ADMIN_TOKEN_ENV} exceeds {MAX_ADMIN_TOKEN_BYTES} bytes"
            )));
        }
        if !token.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(HyperError::new(format!(
                "{ADMIN_TOKEN_ENV} must contain visible ASCII characters only"
            )));
        }
        let mut bytes = [0; MAX_ADMIN_TOKEN_BYTES];
        bytes[..token.len()].copy_from_slice(token.as_bytes());
        Ok(Self {
            bytes,
            length: token.len(),
        })
    }

    /// 固定遍历完整缓冲区比较候选 token。
    fn matches(&self, provided: &str) -> bool {
        let mut candidate = [0; MAX_ADMIN_TOKEN_BYTES];
        let copied = provided.len().min(MAX_ADMIN_TOKEN_BYTES);
        candidate[..copied].copy_from_slice(&provided.as_bytes()[..copied]);
        let mut diff = provided.len() ^ self.length;
        for (left, right) in candidate.iter().zip(self.bytes.iter()) {
            diff |= (left ^ right) as usize;
        }
        diff == 0
    }
}

/// 管理 API 运行配置。`None` 表示管理 API 禁用。
#[derive(Clone)]
pub(crate) struct ManagementConfig {
    /// 鉴权凭据。
    credential: Arc<AdminCredential>,
    /// 监听地址。
    listen: SocketAddr,
}

impl ManagementConfig {
    /// 从环境变量读取管理配置。`HYPERGATE_ADMIN_TOKEN` 未设置时返回 `None`。
    pub(crate) fn from_env() -> HyperResult<Option<Self>> {
        let Some(token) = read_env(ADMIN_TOKEN_ENV) else {
            return Ok(None);
        };
        if token.is_empty() {
            return Err(HyperError::new(format!("{ADMIN_TOKEN_ENV} is empty")));
        }
        let credential = Arc::new(AdminCredential::new(&token)?);
        let listen_str =
            read_env(ADMIN_LISTEN_ENV).unwrap_or_else(|| DEFAULT_ADMIN_LISTEN.to_owned());
        let listen: SocketAddr = listen_str
            .parse()
            .map_err(|e| HyperError::new(format!("invalid {ADMIN_LISTEN_ENV}: {e}")))?;
        if !listen.ip().is_loopback() {
            return Err(HyperError::new(format!(
                "{ADMIN_LISTEN_ENV} must be a loopback address, got {listen}"
            )));
        }
        Ok(Some(Self { credential, listen }))
    }

    /// 返回监听地址。
    pub(crate) fn listen(&self) -> SocketAddr {
        self.listen
    }

    /// 返回鉴权凭据的所有权。
    pub(crate) fn into_credential(self) -> Arc<AdminCredential> {
        self.credential
    }
}

/// 读取环境变量,返回修剪空白后的值。
fn read_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
}

/// 管理 API 共享状态。
#[derive(Clone)]
pub(crate) struct ManagementState {
    /// 生命周期控制入口。
    control: Arc<GatewayControl>,
    /// 鉴权凭据。
    credential: Arc<AdminCredential>,
}

impl ManagementState {
    /// 创建管理 API 共享状态。
    pub(crate) fn new(control: Arc<GatewayControl>, credential: Arc<AdminCredential>) -> Self {
        Self {
            control,
            credential,
        }
    }
}

/// 构建 axum 路由。
pub(crate) fn router(state: ManagementState) -> axum::Router {
    axum::Router::new()
        .route("/api/v1/status", get(status))
        .route("/api/v1/actions/switch", post(switch))
        .route("/api/v1/actions/drain", post(drain))
        .route("/api/v1/actions/stop", post(stop))
        .route("/api/v1/actions/rollback", post(rollback))
        .route("/api/v1/actions/reload", post(reload))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state)
}

/// 启动管理 HTTP 服务,监听到配置地址。返回时表示监听失败。
pub(crate) async fn serve(
    state: ManagementState,
    listen: SocketAddr,
    shutdown: CancellationToken,
) -> HyperResult<()> {
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|e| HyperError::new(format!("admin bind failed: {e}")))?;
    let router = router(state);
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
        .map_err(|e| HyperError::new(format!("admin serve failed: {e}")))
}

/// 校验 Bearer token 鉴权。使用常时比较避免计时侧信道。
fn authorize(headers: &HeaderMap, credential: &AdminCredential) -> bool {
    let Some(header) = headers.get(axum::http::header::AUTHORIZATION) else {
        return false;
    };
    let Ok(value) = header.to_str() else {
        return false;
    };
    let Some(provided) = value.strip_prefix("Bearer ") else {
        return false;
    };
    credential.matches(provided)
}

/// 对全部管理路由和 fallback 统一执行 Bearer 鉴权。
async fn auth_middleware(
    State(state): State<ManagementState>,
    request: Request,
    next: Next,
) -> Response {
    if !authorize(request.headers(), &state.credential) {
        return error_response(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    next.run(request).await
}

/// `GET /api/v1/status`。返回当前 gateway 状态快照。
async fn status(State(state): State<ManagementState>) -> Response {
    match state.control.snapshot() {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(error) => internal_error("read gateway status", error),
    }
}

/// `POST /api/v1/actions/switch`。切换 active version,成功后返回最新快照。
async fn switch(
    State(state): State<ManagementState>,
    body: Result<Json<VersionActionRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request body"),
    };
    let Some(version) = parse_version(request.version) else {
        return error_response(StatusCode::BAD_REQUEST, "version is required");
    };
    match state
        .control
        .switch(version, Some(request.expected_revision))
        .await
    {
        Ok(outcome) => outcome_response(outcome),
        Err(error) => control_error(error),
    }
}

/// 带版本标识的动作请求体(switch 之外的 drain/stop)。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionActionRequest {
    /// 目标版本标识。
    version: String,
    /// 期望配置修订号。
    expected_revision: u64,
}

/// `POST /api/v1/actions/drain`。让非 active 版本进入 draining。
async fn drain(
    State(state): State<ManagementState>,
    body: Result<Json<VersionActionRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request body"),
    };
    let Some(version) = parse_version(request.version) else {
        return error_response(StatusCode::BAD_REQUEST, "version is required");
    };
    match state
        .control
        .drain(version, Some(request.expected_revision))
    {
        Ok(outcome) => outcome_response(outcome),
        Err(error) => control_error(error),
    }
}

/// `POST /api/v1/actions/stop`。在连接清空后停止非 active 版本。
async fn stop(
    State(state): State<ManagementState>,
    body: Result<Json<VersionActionRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request body"),
    };
    let Some(version) = parse_version(request.version) else {
        return error_response(StatusCode::BAD_REQUEST, "version is required");
    };
    match state.control.stop(version, Some(request.expected_revision)) {
        Ok(outcome) => outcome_response(outcome),
        Err(error) => control_error(error),
    }
}

/// 仅包含乐观锁修订号的动作请求体。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RevisionActionRequest {
    /// 期望配置修订号。
    expected_revision: u64,
}

/// `POST /api/v1/actions/rollback`。切回上一个 active version。
async fn rollback(
    State(state): State<ManagementState>,
    body: Result<Json<RevisionActionRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request body"),
    };
    match state
        .control
        .rollback(Some(request.expected_revision))
        .await
    {
        Ok(outcome) => outcome_response(outcome),
        Err(error) => control_error(error),
    }
}

/// `POST /api/v1/actions/reload`。重读配置并保留持久化 active version。
async fn reload(
    State(state): State<ManagementState>,
    body: Result<Json<RevisionActionRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match body {
        Ok(value) => value,
        Err(_) => return error_response(StatusCode::BAD_REQUEST, "invalid request body"),
    };
    match state.control.reload(Some(request.expected_revision)).await {
        Ok(outcome) => outcome_response(outcome),
        Err(error) => control_error(error),
    }
}

/// 把动作完成后的快照读取结果转换成 HTTP 响应。
fn outcome_response(outcome: ControlOutcome) -> Response {
    match outcome.snapshot {
        Ok(snapshot) => Json(snapshot).into_response(),
        Err(error) => {
            eprintln!("gateway admin read status after applied control failed: {error}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "control action applied; status unavailable",
            )
        }
    }
}

/// 未注册管理路由的稳定 JSON 响应。
async fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "not found")
}

/// 已注册路径不支持当前方法时的稳定 JSON 响应。
async fn method_not_allowed() -> Response {
    error_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
}

/// 解析并校验动作中的版本标识。
fn parse_version(version: String) -> Option<VersionId> {
    let version = version.trim();
    if version.is_empty() {
        return None;
    }
    Some(VersionId::new(version))
}

/// 把控制层拒绝转换成稳定冲突响应,详细原因只写服务端日志。
fn control_error(error: ControlError) -> Response {
    match error {
        ControlError::Conflict(message) => {
            eprintln!("gateway admin control rejected: {message}");
            error_response(StatusCode::CONFLICT, "control action rejected")
        }
        ControlError::Internal(error) => internal_error("execute control action", error),
    }
}

/// 把内部错误转换成稳定响应,避免向客户端泄露运行态细节。
fn internal_error(context: &str, error: HyperError) -> Response {
    eprintln!("gateway admin {context} failed: {error}");
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "internal server error")
}

/// 生成稳定 JSON 错误响应。
fn error_response(status: StatusCode, message: &str) -> Response {
    let body = serde_json::json!({ "error": message });
    let mut response = Json(body).into_response();
    *response.status_mut() = status;
    response
}
