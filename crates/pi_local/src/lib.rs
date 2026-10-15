//! Android Local runtime: source-linked Bun execution, native OAuth and HTTP.
pub mod auth;
pub mod auth_fixture;
pub mod bun;
mod bun_vm;
mod dns;
mod fixture;
mod http;
pub mod models;
mod vm;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
/// Thread-safe cancellation owned by the native OAuth task.
#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}
