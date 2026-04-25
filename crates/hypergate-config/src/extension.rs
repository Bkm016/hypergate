//! 配置加载和校验扩展点。

use std::sync::Arc;

use hypergate_core::{ConfigRevision, ExtensionDescriptor, HyperError, HyperResult};

use crate::RuntimeConfig;

/// 配置加载器扩展点。
pub trait ConfigLoader<T>: Send + Sync {
    /// 返回加载器扩展描述。
    fn descriptor(&self) -> ExtensionDescriptor {
        ExtensionDescriptor::new("config.loader", "config-loader", "load runtime config")
    }

    /// 加载并构造一份新的配置。
    fn load(&self, next_revision: ConfigRevision) -> HyperResult<T>;
}

/// 配置校验器扩展点。
pub trait ConfigValidator<T>: Send + Sync {
    /// 返回校验器扩展描述。
    fn descriptor(&self) -> ExtensionDescriptor {
        ExtensionDescriptor::new(
            "config.validator",
            "config-validator",
            "validate runtime config",
        )
    }

    /// 校验配置是否可安全应用。
    fn validate(&self, config: &T) -> HyperResult<()>;
}

/// 配置校验链。
pub struct ConfigValidatorChain<T> {
    /// 按顺序执行的校验器。
    pub validators: Vec<Arc<dyn ConfigValidator<T>>>,
}

impl<T> ConfigValidatorChain<T> {
    /// 创建空校验链。
    pub fn new() -> Self {
        Self {
            validators: Vec::new(),
        }
    }

    /// 追加一个校验器。
    pub fn push(&mut self, validator: Arc<dyn ConfigValidator<T>>) {
        self.validators.push(validator);
    }
}

impl<T> ConfigValidator<T> for ConfigValidatorChain<T>
where
    T: Send + Sync,
{
    /// 返回校验链扩展描述。
    fn descriptor(&self) -> ExtensionDescriptor {
        ExtensionDescriptor::new(
            "config.validator.chain",
            "config-validator",
            "run config validators in order",
        )
    }

    /// 按注册顺序执行全部配置校验器。
    fn validate(&self, config: &T) -> HyperResult<()> {
        // 校验链按注册顺序短路失败,确保后续校验只处理已满足基础约束的配置。
        for validator in &self.validators {
            validator.validate(config)?;
        }
        Ok(())
    }
}

impl<T> Default for ConfigValidatorChain<T> {
    /// 默认创建空校验链。
    fn default() -> Self {
        Self::new()
    }
}

/// 不做外部文件读取的固定配置加载器。
pub struct StaticConfigLoader<T> {
    /// 作为模板返回的配置。
    pub template: T,
}

impl<T> ConfigLoader<T> for StaticConfigLoader<T>
where
    T: Clone + Send + Sync,
{
    /// 返回固定配置加载器扩展描述。
    fn descriptor(&self) -> ExtensionDescriptor {
        ExtensionDescriptor::new(
            "config.loader.static",
            "config-loader",
            "load config from in-memory template",
        )
    }

    /// 基于模板返回一份配置副本。
    fn load(&self, _next_revision: ConfigRevision) -> HyperResult<T> {
        Ok(self.template.clone())
    }
}

/// 默认配置校验器。
pub struct DefaultConfigValidator;

impl ConfigValidator<RuntimeConfig> for DefaultConfigValidator {
    /// 返回默认校验器扩展描述。
    fn descriptor(&self) -> ExtensionDescriptor {
        ExtensionDescriptor::new(
            "config.validator.default",
            "config-validator",
            "validate required hypergate config",
        )
    }

    /// 校验 gateway 运行需要的最小配置约束。
    fn validate(&self, config: &RuntimeConfig) -> HyperResult<()> {
        // 默认校验器只检查底座运行所需的最小约束,应用约束应通过额外校验器扩展。
        if config.active_version.value.is_empty() {
            return Err(HyperError::new("active version is empty"));
        }
        if !config.versions.contains_key(&config.active_version) {
            return Err(HyperError::new("active version is not configured"));
        }
        for (id, version) in &config.versions {
            if id.value.is_empty() {
                return Err(HyperError::new("version id is empty"));
            }
            if version.endpoint.is_empty() {
                return Err(HyperError::new("version endpoint is empty"));
            }
        }
        Ok(())
    }
}
