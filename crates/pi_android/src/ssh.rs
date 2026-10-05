//! SSH from the phone. The phone has its own key, made on first use and kept in
//! the app's private storage; the computer's host key is remembered the first
//! time and must match after that. One connection carries every command.
//!
//! The SSH library runs on its own Tokio runtime; GPUI awaits its results
//! through channels, so nothing here blocks the UI thread.

use anyhow::{Context as _, Result, anyhow, bail};
use russh::{
    ChannelMsg,
    client::{self, Handle},
    keys::{Algorithm, HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use serde_json::Value;
use std::{
    fmt,
    future::Future,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

/// Where SSH connects: `user@host`, optionally `:port`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub user: String,
    pub host: String,
    pub port: u16,
}

impl Address {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        let Some((user, rest)) = text.split_once('@') else {
            bail!("Add your user name on the computer, like you@studio-mac.local.");
        };
        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (
                host,
                port.parse()
                    .map_err(|_| anyhow!("The port after “:” should be a number."))?,
            ),
            _ => (rest, 22),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if user.is_empty() || host.is_empty() || text.contains(char::is_whitespace) {
            bail!("Enter the computer's SSH address, like you@studio-mac.local.");
        }
        Ok(Self {
            user: user.into(),
            host: host.into(),
            port,
        })
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}@{}", self.user, self.host)?;
        if self.port != 22 {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

/// The phone's key.
#[derive(Clone)]
pub struct Identity {
    key: Arc<PrivateKey>,
}

impl Identity {
    /// Reads the key at `path`, or makes one and keeps it there.
    pub fn load_or_create(path: &Path) -> Result<Self> {
        if let Ok(text) = std::fs::read_to_string(path) {
            let key = PrivateKey::from_openssh(&text).context("The phone's key is unreadable")?;
            return Ok(Self { key: Arc::new(key) });
        }
        let mut key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519)
            .map_err(|error| anyhow!("Could not make a key: {error}"))?;
        key.set_comment("pi-phone");
        let text = key
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .map_err(|error| anyhow!("Could not save the key: {error}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let partial = path.with_extension("partial");
        write_private(&partial, text.as_bytes())?;
        std::fs::rename(&partial, path)?;
        Ok(Self { key: Arc::new(key) })
    }

    /// One line for `~/.ssh/authorized_keys`.
    pub fn public_line(&self) -> String {
        self.key
            .public_key()
            .to_openssh()
            .unwrap_or_else(|_| "ssh-ed25519".into())
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)?.write_all(bytes)?;
    Ok(())
}

/// Why connecting failed, in terms the connect screen can act on.
#[derive(Debug)]
pub enum Failure {
    /// The computer turned down the phone's key: it isn't in authorized_keys.
    KeyRefused,
    /// The computer's host key isn't the one seen before.
    HostKeyChanged {
        seen: String,
        now: String,
    },
    Unreachable(String),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::KeyRefused => write!(
                f,
                "The computer didn't accept this phone's key. Add it to ~/.ssh/authorized_keys on the computer."
            ),
            Self::HostKeyChanged { seen, now } => write!(
                f,
                "This computer's identity changed (was {seen}, now {now}). If you reinstalled it, forget it in Settings and connect again."
            ),
            Self::Unreachable(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for Failure {}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("pi-ssh")
            .enable_all()
            .build()
            .expect("the SSH runtime starts")
    })
}

/// Runs `work` on the SSH runtime; await the result from any executor.
async fn on_runtime<T: Send + 'static>(
    work: impl Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    let (sender, receiver) = async_channel::bounded(1);
    runtime().spawn(async move {
        let _closed = sender.send(work.await).await;
    });
    receiver
        .recv()
        .await
        .context("The connection stopped unexpectedly")?
}

/// Runs `work` on the SSH runtime without waiting for it.
pub fn spawn(work: impl Future<Output = ()> + Send + 'static) {
    runtime().spawn(work);
}

struct Client {
    known: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = anyhow::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            bail!("The computer offered a certificate; use a plain host key");
        };
        let now = key.fingerprint(HashAlg::Sha256).to_string();
        if let Some(seen) = &self.known
            && *seen != now
        {
            return Err(Failure::HostKeyChanged {
                seen: seen.clone(),
                now,
            }
            .into());
        }
        *self.seen.lock().unwrap() = Some(now);
        Ok(true)
    }
}

/// An open connection to a computer.
#[derive(Clone)]
pub struct Connection {
    handle: Arc<Handle<Client>>,
    /// The computer's host key fingerprint, `SHA256:…`.
    pub fingerprint: String,
}

/// What a command printed, and how it exited.
#[derive(Debug)]
pub struct Output {
    pub status: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

/// A running command that speaks JSON lines: records in, records out.
pub struct Pipe {
    pub records: async_channel::Receiver<Value>,
    pub input: async_channel::Sender<Value>,
    /// Why it ended: what it printed on stderr, or how it exited.
    pub ended: async_channel::Receiver<String>,
}

impl Connection {
    /// Connects and signs in with the phone's key. `known` is the host key
    /// fingerprint seen before, if any.
    pub async fn open(address: Address, identity: Identity, known: Option<String>) -> Result<Self> {
        on_runtime(async move {
            let config = Arc::new(client::Config {
                keepalive_interval: Some(Duration::from_secs(15)),
                keepalive_max: 3,
                inactivity_timeout: None,
                nodelay: true,
                ..Default::default()
            });
            let seen = Arc::new(Mutex::new(None));
            let client = Client {
                known,
                seen: seen.clone(),
            };
            let target = (address.host.as_str(), address.port);
            let connecting = client::connect(config, target, client);
            let mut handle = match tokio::time::timeout(Duration::from_secs(15), connecting).await
            {
                Err(_) => {
                    return Err(Failure::Unreachable(format!(
                        "{} didn't answer. Check that it's awake, on this network, and has Remote Login on.",
                        address.host
                    ))
                    .into());
                }
                Ok(Err(error)) => return Err(unreachable(&address, error)),
                Ok(Ok(handle)) => handle,
            };
            let hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
            let signed_in = handle
                .authenticate_publickey(
                    address.user.clone(),
                    PrivateKeyWithHashAlg::new(identity.key.clone(), hash),
                )
                .await?;
            if !signed_in.success() {
                return Err(Failure::KeyRefused.into());
            }
            let fingerprint = seen.lock().unwrap().clone().unwrap_or_default();
            Ok(Self {
                handle: Arc::new(handle),
                fingerprint,
            })
        })
        .await
    }

    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Runs `command` to the end.
    pub async fn run(&self, command: String) -> Result<Output> {
        let handle = self.handle.clone();
        on_runtime(async move {
            let mut channel = handle.channel_open_session().await?;
            channel.exec(true, command).await?;
            let (mut stdout, mut stderr, mut status) = (Vec::new(), Vec::new(), None);
            let collect = async {
                while let Some(message) = channel.wait().await {
                    match message {
                        ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                        ChannelMsg::ExtendedData { data, .. } => stderr.extend_from_slice(&data),
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        _ => {}
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(60), collect)
                .await
                .context("The computer took too long to answer")?;
            Ok(Output {
                status,
                stdout: String::from_utf8_lossy(&stdout).into_owned(),
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            })
        })
        .await
    }

    /// Starts `command` and exchanges JSON lines with it until either side ends.
    pub async fn pipe(&self, command: String) -> Result<Pipe> {
        let handle = self.handle.clone();
        let (records_out, records) = async_channel::bounded::<Value>(1024);
        let (input, input_in) = async_channel::unbounded::<Value>();
        let (ended_out, ended) = async_channel::bounded::<String>(1);
        on_runtime(async move {
            let channel = handle.channel_open_session().await?;
            channel.exec(true, command).await?;
            let (mut reader, writer) = channel.split();
            tokio::spawn(async move {
                while let Ok(record) = input_in.recv().await {
                    let mut line = record.to_string().into_bytes();
                    line.push(b'\n');
                    if writer.data_bytes(line).await.is_err() {
                        break;
                    }
                }
                let _ = writer.eof().await;
            });
            tokio::spawn(async move {
                let (mut pending, mut stderr, mut status) = (Vec::new(), Vec::new(), None);
                while let Some(message) = reader.wait().await {
                    match message {
                        ChannelMsg::Data { data } => {
                            pending.extend_from_slice(&data);
                            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                                let line: Vec<u8> = pending.drain(..=end).collect();
                                match serde_json::from_slice::<Value>(&line) {
                                    Ok(record) => {
                                        if records_out.send(record).await.is_err() {
                                            return;
                                        }
                                    }
                                    Err(error) => {
                                        log::warn!("Skipping a line that isn't JSON: {error}")
                                    }
                                }
                            }
                        }
                        ChannelMsg::ExtendedData { data, .. } if stderr.len() < 8192 => {
                            stderr.extend_from_slice(&data)
                        }
                        ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                        _ => {}
                    }
                }
                let stderr = String::from_utf8_lossy(&stderr).trim().to_owned();
                let reason = match (stderr.is_empty(), status) {
                    (false, _) => stderr,
                    (true, Some(status)) => format!("It exited with status {status}"),
                    (true, None) => "The connection closed".into(),
                };
                let _ = ended_out.send(reason).await;
            });
            Ok(())
        })
        .await?;
        Ok(Pipe {
            records,
            input,
            ended,
        })
    }
}

fn unreachable(address: &Address, error: anyhow::Error) -> anyhow::Error {
    if error.downcast_ref::<Failure>().is_some() {
        return error;
    }
    let detail = format!("{error:#}");
    let hint = if address.host.ends_with(".local") {
        " If the phone can't find .local names, use the computer's IP address or its Tailscale name."
    } else {
        ""
    };
    Failure::Unreachable(format!("Couldn't reach {}: {detail}.{hint}", address.host)).into()
}

/// Quotes `text` for a POSIX shell, and for zsh, bash and fish alike.
pub fn quote(text: &str) -> Result<String> {
    if text.contains(['\'', '\n', '\r', '\0', '\\']) {
        bail!("Unexpected characters in {text:?}");
    }
    Ok(format!("'{text}'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_need_a_user_and_take_a_port() {
        let address = Address::parse("nick@studio-mac.local").unwrap();
        assert_eq!(
            (address.user.as_str(), address.host.as_str(), address.port),
            ("nick", "studio-mac.local", 22)
        );
        assert_eq!(Address::parse("dev@10.0.4.12:2222").unwrap().port, 2222);
        assert_eq!(Address::parse("dev@[fe80::1]").unwrap().host, "fe80::1");
        assert_eq!(
            Address::parse("dev@10.0.4.12:2222").unwrap().to_string(),
            "dev@10.0.4.12:2222"
        );
        assert!(Address::parse("studio-mac.local").is_err());
        assert!(Address::parse("nick@studio mac").is_err());
        assert!(Address::parse("nick@host:ssh").is_err());
    }

    #[test]
    fn the_key_is_made_once_and_kept() {
        let dir = std::env::temp_dir().join(format!("pi-android-key-{}", std::process::id()));
        let path = dir.join("id_ed25519");
        let made = Identity::load_or_create(&path).unwrap();
        let line = made.public_line();
        assert!(line.starts_with("ssh-ed25519 ") && line.ends_with(" pi-phone"));
        assert_eq!(Identity::load_or_create(&path).unwrap().public_line(), line);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn quoting_refuses_what_it_cannot_quote_for_every_shell() {
        assert_eq!(
            quote("/Users/nick/.pi/desktop/bin/ab/pi-desktop-remote").unwrap(),
            "'/Users/nick/.pi/desktop/bin/ab/pi-desktop-remote'"
        );
        assert!(quote("/tmp/it's").is_err());
        assert!(quote("/tmp/a\\b").is_err());
    }
}
