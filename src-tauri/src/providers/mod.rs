//! 用量 Provider 适配层。
#![allow(dead_code)]

pub mod cursor;
pub mod deepseek;

/// 用量 Provider 接口骨架（后续实现真实取数）。
pub trait UsageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
}
