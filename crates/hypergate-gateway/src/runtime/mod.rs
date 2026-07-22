//! 多版本运行时、切换和长连接排水。

#![deny(missing_docs)]

mod lease;
mod registry;
mod version;

pub(crate) use lease::VersionLease;
pub(crate) use registry::VersionRegistry;
pub(crate) use version::{VersionRuntime, VersionSnapshot};
