use std::fmt::{Display, Formatter};
use std::sync::Arc;

/// HyperGate 统一错误类型。
#[derive(Debug, Clone)]
pub struct HyperError {
    /// 面向调用方和日志的错误信息。
    pub message: Arc<str>,
}

impl HyperError {
    /// 创建一个新的错误值。
    pub fn new(message: impl Into<Arc<str>>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for HyperError {
    /// 直接输出错误消息。
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HyperError {}

/// HyperGate 通用结果类型。
pub type HyperResult<T> = Result<T, HyperError>;
