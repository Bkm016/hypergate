/// 配置快照修订号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigRevision {
    /// 单调递增的修订值。
    pub value: u64,
}

impl ConfigRevision {
    /// 初始配置修订号。
    pub const INITIAL: Self = Self { value: 1 };

    /// 生成下一个配置修订号。
    pub fn next(self) -> Self {
        Self {
            value: self.value + 1,
        }
    }
}

/// 请求连接类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// 普通短请求。
    Unary,
    /// SSE 或 chunked 流式响应。
    Stream,
}
