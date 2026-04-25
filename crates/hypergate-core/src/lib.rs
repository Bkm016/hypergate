//! HyperGate 核心类型和扩展边界。

#![deny(missing_docs)]

mod error;
mod extension;
mod request;
mod version;

pub use error::{HyperError, HyperResult};
pub use extension::{DescribedExtension, ExtensionDescriptor, ExtensionRegistry};
pub use request::{ConfigRevision, RequestKind};
pub use version::{VersionId, VersionState};
