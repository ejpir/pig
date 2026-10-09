//! A prompt is a complete payload with its own stable admission identity, not
//! just a string. Equal text with different images represents different work.
use pi_core::protocol::ImageContent;
use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

static REQUEST_PREFIX: OnceLock<u64> = OnceLock::new();
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

/// Unique for this app process, with a random prefix making reuse after restart
/// negligibly unlikely. One sequence supplies both durable admission identities
/// and disposable transport correlations, so they cannot collide in-process.
pub(crate) fn request_id() -> String {
    let prefix = *REQUEST_PREFIX.get_or_init(rand::random);
    let sequence = NEXT_REQUEST
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("request identity space exhausted");
    format!("phone-{prefix:016x}-{sequence:016x}")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub request_id: String,
    pub message: String,
    pub images: Vec<ImageContent>,
}

impl Prompt {
    pub fn new(message: String, images: Vec<ImageContent>) -> Self {
        Self {
            request_id: request_id(),
            message,
            images,
        }
    }

    pub fn label(&self) -> String {
        if self.images.is_empty() {
            self.message.clone()
        } else {
            format!(
                "{}{}{} image{}",
                self.message,
                if self.message.is_empty() { "" } else { " · " },
                self.images.len(),
                if self.images.len() == 1 { "" } else { "s" }
            )
        }
    }
}

impl From<String> for Prompt {
    fn from(message: String) -> Self {
        Self::new(message, Vec::new())
    }
}
impl From<&str> for Prompt {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn request_identities_are_unique_and_backend_safe() {
        let ids: HashSet<_> = (0..10_000).map(|_| request_id()).collect();
        assert_eq!(ids.len(), 10_000);
        assert!(ids.iter().all(|id| {
            id.len() <= 128
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        }));
        let prompt = Prompt::new("hello".into(), Vec::new());
        assert!(!ids.contains(&prompt.request_id));
    }
}
