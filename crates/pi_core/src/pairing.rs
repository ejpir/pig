//! Versioned, compact QR payloads for enrolling a phone through SSH itself.
//! The QR carries a short-lived bootstrap seed, never a lasting phone or host key.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: u32 = 1;
pub const URL_PREFIX: &str = "pi://pair/v1#";
pub const MAX_OFFER_AGE: u64 = 10 * 60;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    pub version: u32,
    pub id: String,
    pub user: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub host_keys: Vec<String>,
    /// URL-safe base64 for a 32-byte, one-use Ed25519 seed.
    pub bootstrap_seed: String,
    /// Unix seconds. The computer is authoritative; the phone checks grossly stale codes.
    pub expires_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub version: u32,
    pub id: String,
    pub public_key: String,
    pub device_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperCapabilities {
    #[serde(default)]
    pub pi: bool,
    #[serde(default)]
    pub durable: bool,
    #[serde(default)]
    pub durable_experimental: bool,
    #[serde(default)]
    pub watchers: bool,
    #[serde(default)]
    pub sessions: bool,
    #[serde(default)]
    pub directories: bool,
    #[serde(default)]
    pub commands: bool,
    #[serde(default)]
    pub jj_history: bool,
    #[serde(default)]
    pub delete_sessions: bool,
    #[serde(default)]
    pub image_prompts: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Helper {
    pub path: String,
    pub home: String,
    pub images: bool,
    pub gateway: bool,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub capabilities: HelperCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub version: u32,
    pub approved: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub helper: Option<Helper>,
}

impl Offer {
    pub fn url(&self) -> Result<String> {
        self.validate()?;
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?);
        Ok(format!("{URL_PREFIX}{encoded}"))
    }

    pub fn parse(url: &str) -> Result<Self> {
        let encoded = url
            .strip_prefix(URL_PREFIX)
            .context("This isn't a Pi computer pairing code")?;
        if encoded.is_empty() || encoded.len() > 16 * 1024 {
            bail!("The pairing code has an invalid size");
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .context("The pairing code is damaged")?;
        let offer: Self = serde_json::from_slice(&bytes).context("The pairing code is damaged")?;
        offer.validate()?;
        Ok(offer)
    }

    pub fn seed(&self) -> Result<[u8; 32]> {
        let bytes = URL_SAFE_NO_PAD
            .decode(&self.bootstrap_seed)
            .context("The pairing credential is damaged")?;
        bytes
            .try_into()
            .map_err(|_| anyhow::anyhow!("The pairing credential has the wrong size"))
    }

    pub fn validate(&self) -> Result<()> {
        if self.version != VERSION {
            bail!("This pairing code needs a different Pi app version");
        }
        if self.id.len() != 32 || !self.id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("The pairing code has an invalid identity");
        }
        valid_part(&self.user, 128, "user")?;
        if self.user.contains('@') {
            bail!("The pairing code has an invalid user");
        }
        if self.hosts.is_empty() || self.hosts.len() > 8 {
            bail!("The pairing code has no usable computer address");
        }
        for host in &self.hosts {
            valid_part(host, 255, "computer address")?;
            if host.contains(['/', '@']) {
                bail!("The pairing code has an invalid computer address");
            }
        }
        if self.port == 0 {
            bail!("The pairing code has an invalid SSH port");
        }
        if self.host_keys.is_empty()
            || self.host_keys.len() > 8
            || self
                .host_keys
                .iter()
                .any(|key| !key.starts_with("SHA256:") || key.len() > 128)
        {
            bail!("The pairing code cannot verify the computer");
        }
        self.seed()?;
        Ok(())
    }

    pub fn check_time(&self) -> Result<()> {
        let now = now();
        if self.expires_at.saturating_add(30) < now {
            bail!("This pairing code expired; make a new one on the computer");
        }
        if self.expires_at > now.saturating_add(MAX_OFFER_AGE) {
            bail!("The pairing code's clock is too far from this phone");
        }
        Ok(())
    }
}

impl Request {
    pub fn new(id: String, public_key: String, device_name: String) -> Self {
        Self {
            version: VERSION,
            id,
            public_key,
            device_name,
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// A human-checkable code derived from this exact offer and permanent phone key.
pub fn confirmation_code(id: &str, public_key: &str) -> String {
    let digest = Sha256::digest(format!("pi-pair-v1\0{id}\0{public_key}").as_bytes());
    let number = u32::from_be_bytes(digest[..4].try_into().unwrap()) % 1_000_000;
    format!("{:03} {:03}", number / 1000, number % 1000)
}

fn valid_part(text: &str, maximum: usize, name: &str) -> Result<()> {
    if text.is_empty()
        || text.len() > maximum
        || text
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        bail!("The pairing code has an invalid {name}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer() -> Offer {
        Offer {
            version: VERSION,
            id: "0123456789abcdef0123456789abcdef".into(),
            user: "nick".into(),
            hosts: vec!["studio.local".into(), "192.168.1.2".into()],
            port: 22,
            host_keys: vec!["SHA256:computer".into()],
            bootstrap_seed: URL_SAFE_NO_PAD.encode([7; 32]),
            expires_at: now() + 120,
        }
    }

    #[test]
    fn qr_offer_round_trips_without_escaping() {
        let offer = offer();
        let url = offer.url().unwrap();
        assert!(url.starts_with(URL_PREFIX));
        assert!(!url[URL_PREFIX.len()..].contains(['+', '/', '=']));
        assert_eq!(Offer::parse(&url).unwrap(), offer);
        assert_eq!(offer.seed().unwrap(), [7; 32]);
    }

    #[test]
    fn malformed_or_untrusted_offers_are_rejected() {
        assert!(Offer::parse("https://example.com").is_err());
        let mut broken = offer();
        broken.host_keys.clear();
        assert!(broken.url().is_err());
        let mut broken = offer();
        broken.user = "bad user".into();
        assert!(broken.url().is_err());
        let mut expired = offer();
        expired.expires_at = now() - 31;
        assert!(expired.check_time().is_err());
    }

    #[test]
    fn confirmation_code_is_stable_and_key_specific() {
        assert_eq!(
            confirmation_code("pair", "ssh-ed25519 first"),
            confirmation_code("pair", "ssh-ed25519 first")
        );
        assert_ne!(
            confirmation_code("pair", "ssh-ed25519 first"),
            confirmation_code("pair", "ssh-ed25519 second")
        );
    }

    #[test]
    fn older_helper_discovery_defaults_new_metadata() {
        let helper: Helper = serde_json::from_str(
            r#"{"path":"/home/me/pi-desktop-remote","home":"/home/me","images":true,"gateway":true}"#,
        )
        .unwrap();
        assert_eq!(helper.release, None);
        assert_eq!(helper.protocol, None);
        assert_eq!(helper.platform, None);
        assert_eq!(helper.capabilities, HelperCapabilities::default());

        let encoded = serde_json::to_value(HelperCapabilities {
            durable_experimental: true,
            image_prompts: true,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(encoded["durableExperimental"], true);
        assert_eq!(encoded["imagePrompts"], true);
    }
}
