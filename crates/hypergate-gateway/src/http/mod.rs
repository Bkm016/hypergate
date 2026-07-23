//! HTTP 入口和版本反向代理。

#![deny(missing_docs)]

mod body;
mod client;
mod extension;
mod gateway;
mod proxy;
mod stream;

pub(crate) use body::ProxyBodyPolicy;
pub(crate) use client::{HealthChecker, VersionClients};
pub(crate) use extension::{DefaultRequestKindClassifier, RequestKindClassifier};
pub(crate) use gateway::{Gateway, HttpState};
pub(crate) use proxy::serve;
