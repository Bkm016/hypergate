//! 配置快照管理器。请求侧读取快照,控制侧原子替换快照。

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use hypergate_core::{ConfigRevision, HyperResult};

use crate::{ConfigLoader, ConfigValidator, ReloadTrigger, StaticConfigLoader};

/// 运行时配置管理器。
pub struct ConfigManager<T> {
    /// 当前配置快照。
    pub current: ArcSwap<T>,
    /// 最近一次成功配置的加载器。
    pub loader: Arc<dyn ConfigLoader<T>>,
    /// 配置校验器。
    pub validator: Arc<dyn ConfigValidator<T>>,
    /// manager 内部重载修订号。
    revision: AtomicU64,
}

impl<T> ConfigManager<T>
where
    T: Send + Sync + 'static,
{
    /// 创建配置管理器。
    pub fn new(
        initial: T,
        loader: Arc<dyn ConfigLoader<T>>,
        validator: Arc<dyn ConfigValidator<T>>,
    ) -> Self {
        Self::with_revision(initial, loader, validator, ConfigRevision::INITIAL)
    }

    /// 使用外部持久化状态中的修订号创建配置管理器。
    pub fn with_revision(
        initial: T,
        loader: Arc<dyn ConfigLoader<T>>,
        validator: Arc<dyn ConfigValidator<T>>,
        revision: ConfigRevision,
    ) -> Self {
        Self {
            current: ArcSwap::from_pointee(initial),
            loader,
            validator,
            revision: AtomicU64::new(revision.value),
        }
    }

    /// 使用固定配置模板和闭包校验器创建配置管理器。
    ///
    /// 该构造器面向 version app 这类轻量业务配置。调用方只需要提供
    /// 初始配置和校验函数,无需手写 loader / validator 类型。
    pub fn static_config<F>(initial: T, validate: F) -> Self
    where
        T: Clone,
        F: Fn(&T) -> HyperResult<()> + Send + Sync + 'static,
    {
        Self::new(
            initial.clone(),
            Arc::new(StaticConfigLoader { template: initial }),
            Arc::new(FunctionConfigValidator { validate }),
        )
    }

    /// 获取当前配置快照。请求热路径无锁读取,请求侧可持有该快照直到结束。
    pub fn snapshot(&self) -> Arc<T> {
        self.current.load_full()
    }

    /// 返回最近一次成功应用的配置修订号。
    pub fn revision(&self) -> ConfigRevision {
        ConfigRevision {
            value: self.revision.load(Ordering::Relaxed),
        }
    }

    /// 触发一次配置重载。新配置校验失败时保留旧配置。
    pub fn reload(&self, _trigger: ReloadTrigger) -> HyperResult<Arc<T>> {
        let next_revision = self.revision().next();
        let next = self.loader.load(next_revision)?;
        self.validator.validate(&next)?;

        // 新配置必须先完整加载并通过校验,再一次性替换当前快照。
        let next = Arc::new(next);
        self.current.store(next.clone());
        self.revision.store(next_revision.value, Ordering::Relaxed);
        Ok(next)
    }

    /// 直接应用已经构建完成的配置。用于管理指令产生的运行态变更。
    pub fn apply(&self, next: T) -> HyperResult<Arc<T>> {
        self.validator.validate(&next)?;
        Ok(self.apply_validated(next))
    }

    /// 提交调用方已经通过本管理器 validator 校验的配置。
    /// 该入口用于先持久化外部运行状态、再无失败替换内存快照的控制事务。
    pub fn apply_validated(&self, next: T) -> Arc<T> {
        let next = Arc::new(next);
        self.current.store(next.clone());
        self.revision.fetch_add(1, Ordering::Relaxed);
        next
    }
}

/// 闭包配置校验器。
struct FunctionConfigValidator<F> {
    /// 调用方提供的校验函数。
    validate: F,
}

impl<T, F> ConfigValidator<T> for FunctionConfigValidator<F>
where
    F: Fn(&T) -> HyperResult<()> + Send + Sync,
{
    /// 执行调用方提供的校验函数。
    fn validate(&self, config: &T) -> HyperResult<()> {
        (self.validate)(config)
    }
}
