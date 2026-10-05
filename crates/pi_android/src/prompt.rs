//! A prompt is a complete payload with its own stable admission identity, not
//! just a string. Equal text with different images represents different work.
use pi_core::protocol::ImageContent;

#[derive(Clone, Debug, PartialEq)]
pub struct Prompt {
    pub request_id: String,
    pub message: String,
    pub images: Vec<ImageContent>,
}

impl Prompt {
    pub fn new(message: String, images: Vec<ImageContent>) -> Self {
        Self {
            request_id: format!("phone-{:016x}", rand::random::<u64>()),
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
