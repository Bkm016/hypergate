use crate::{HyperError, HyperResult};

/// 扩展点描述。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensionDescriptor {
    /// 扩展唯一标识。
    pub id: &'static str,
    /// 扩展类别。
    pub kind: &'static str,
    /// 扩展用途说明。
    pub description: &'static str,
}

impl ExtensionDescriptor {
    /// 创建稳定的扩展描述。
    pub const fn new(id: &'static str, kind: &'static str, description: &'static str) -> Self {
        Self {
            id,
            kind,
            description,
        }
    }
}

/// 可被注册表识别的扩展。
pub trait DescribedExtension: Send + Sync {
    /// 返回扩展描述。
    fn descriptor(&self) -> ExtensionDescriptor;
}

/// 通用扩展注册表。
pub struct ExtensionRegistry<T> {
    /// 已注册扩展。
    pub entries: Vec<T>,
}

impl<T> ExtensionRegistry<T> {
    /// 创建空注册表。
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// 创建带初始扩展的注册表。
    pub fn from_entries(entries: Vec<T>) -> Self {
        Self { entries }
    }

    /// 遍历全部扩展。
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter()
    }

    /// 返回扩展数量。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 判断注册表是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<T> ExtensionRegistry<T>
where
    T: DescribedExtension,
{
    /// 注册扩展。重复 id 会被拒绝,避免中心表被后注册项静默覆盖。
    pub fn register(&mut self, extension: T) -> HyperResult<()> {
        let descriptor = extension.descriptor();
        if self.find(descriptor.id).is_some() {
            return Err(HyperError::new(format!(
                "extension already registered: {}",
                descriptor.id
            )));
        }
        self.entries.push(extension);
        Ok(())
    }

    /// 按扩展 id 查找扩展。
    pub fn find(&self, id: &str) -> Option<&T> {
        self.entries
            .iter()
            .find(|extension| extension.descriptor().id == id)
    }
}

impl<T> Default for ExtensionRegistry<T> {
    /// 默认创建空扩展注册表。
    fn default() -> Self {
        Self::new()
    }
}

/// 用表达式列表构建扩展注册表。
#[macro_export]
macro_rules! hypergate_extension_registry {
    ($($extension:expr),* $(,)?) => {{
        let mut registry = $crate::ExtensionRegistry::new();
        $(registry.register($extension)?;)*
        registry
    }};
}
