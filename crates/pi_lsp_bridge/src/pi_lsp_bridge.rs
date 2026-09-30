//! Feeds the desktop's language-server errors back to pi; pi itself has no LSP support.
//!
//! Each session's pi loads `extension/pi-desktop-lsp.ts` (`pi -e`), which asks this
//! session's bridge after `edit` and `write` results and when a run ends. The
//! [`Checker`] answers from the session folder's Zed `Project`, and only once the
//! user has started language services there; otherwise every answer is empty.
//!
//! The same extension and socket carry the session's other requests (jj snapshots
//! around `bash`, pi's jj tools): anything that is not `file` or `run_end` goes to
//! the session's [`Handler`].
mod check;
#[cfg(all(test, feature = "fake-lsp"))]
mod fake_lsp_tests;
#[cfg(test)]
mod tests;

use anyhow::{Context as _, Result};
pub use check::{Checker, FILE_WAIT, RUN_WAIT, Request};
use gpui::{App, AppContext as _, AsyncApp, Entity, Task, WeakEntity};
use project::Project;
use smol::io::{AsyncRead, AsyncWrite};
use std::{
    ffi::OsString,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

/// The extension pi loads; it does nothing without [`ADDRESS_ENV`].
pub const EXTENSION: &str = include_str!("../extension/pi-desktop-lsp.ts");
/// Where the extension connects: a socket path, or `tcp:<port>` on 127.0.0.1.
pub const ADDRESS_ENV: &str = "PI_DESKTOP_LSP";
/// With a port: the token each request must carry.
pub const TOKEN_ENV: &str = "PI_DESKTOP_LSP_TOKEN";

#[derive(Clone, Copy, Debug)]
enum Transport {
    /// A socket in a private directory: only this user can reach it.
    #[cfg(unix)]
    Unix,
    /// A loopback port, which any local program can reach, so requests carry a
    /// random token. Windows has no Unix sockets in Rust's standard library.
    // Elsewhere only the tests use it, which keeps this path tested on Linux.
    #[cfg_attr(unix, allow(dead_code))]
    Tcp,
}

#[cfg(unix)]
const TRANSPORT: Transport = Transport::Unix;
#[cfg(not(unix))]
const TRANSPORT: Transport = Transport::Tcp;

/// Answers the session's other requests, as `{"text"}`; the session owns them.
pub type Handler = Rc<dyn Fn(serde_json::Value, &mut App) -> Task<Result<String>>>;

/// One session's listener. Dropping it stops listening and removes a socket file.
pub struct Bridge {
    env: Vec<(&'static str, OsString)>,
    extension: PathBuf,
    _socket_dir: Option<tempfile::TempDir>,
    _checker: Entity<Checker>,
    _listener: Task<()>,
}

impl Bridge {
    /// `project` returns the session folder's project while its language services
    /// are on; `allow` says whether the user wants a kind of check.
    pub fn start(
        project: impl Fn(&App) -> Option<Entity<Project>> + 'static,
        allow: impl Fn(&Request, &App) -> bool + 'static,
        handler: Handler,
        cx: &mut App,
    ) -> Result<Self> {
        let data = dirs::data_local_dir().context("Cannot locate desktop data directory")?;
        Self::start_in(
            project,
            allow,
            handler,
            &data.join("pi-desktop"),
            TRANSPORT,
            cx,
        )
    }

    /// Like [`Bridge::start`], writing the extension into `extension_dir`.
    fn start_in(
        project: impl Fn(&App) -> Option<Entity<Project>> + 'static,
        allow: impl Fn(&Request, &App) -> bool + 'static,
        handler: Handler,
        extension_dir: &Path,
        transport: Transport,
        cx: &mut App,
    ) -> Result<Self> {
        let checker = cx.new(|_| Checker::new(project).allowing(allow));
        let weak = checker.downgrade();
        let (env, socket_dir, listener) = match transport {
            #[cfg(unix)]
            Transport::Unix => {
                let dir = tempfile::Builder::new()
                    .prefix("pi-desktop-lsp")
                    .tempdir()?;
                let socket = dir.path().join("socket");
                let listener = smol::net::unix::UnixListener::bind(&socket)?;
                let task = cx.spawn(async move |cx| {
                    while let Ok((stream, _)) = listener.accept().await {
                        serve_later(stream, weak.clone(), handler.clone(), None, cx);
                    }
                });
                (vec![(ADDRESS_ENV, socket.into())], Some(dir), task)
            }
            Transport::Tcp => {
                let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
                let port = listener.local_addr()?.port();
                let listener = smol::net::TcpListener::try_from(listener)?;
                let token: Arc<str> = format!("{:032x}", rand::random::<u128>()).into();
                let env = vec![
                    (ADDRESS_ENV, format!("tcp:{port}").into()),
                    (TOKEN_ENV, token.to_string().into()),
                ];
                let task = cx.spawn(async move |cx| {
                    while let Ok((stream, _)) = listener.accept().await {
                        serve_later(
                            stream,
                            weak.clone(),
                            handler.clone(),
                            Some(token.clone()),
                            cx,
                        );
                    }
                });
                (env, None, task)
            }
        };
        let extension = install_extension(extension_dir)?;
        Ok(Self {
            env,
            extension,
            _socket_dir: socket_dir,
            _checker: checker,
            _listener: listener,
        })
    }

    /// Pass to pi as `-e <extension>`.
    pub fn extension(&self) -> &Path {
        &self.extension
    }

    /// Add to pi's environment: [`ADDRESS_ENV`], and [`TOKEN_ENV`] with a port.
    pub fn env(&self) -> &[(&'static str, OsString)] {
        &self.env
    }
}

/// Writes the extension where pi can load it, once per content change.
fn install_extension(dir: &Path) -> Result<PathBuf> {
    let path = dir.join("pi-desktop-lsp.ts");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(EXTENSION) {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&path, EXTENSION)?;
    }
    Ok(path)
}

fn serve_later<S>(
    stream: S,
    checker: WeakEntity<Checker>,
    handler: Handler,
    token: Option<Arc<str>>,
    cx: &mut AsyncApp,
) where
    S: AsyncRead + AsyncWrite + Clone + Unpin + 'static,
{
    cx.spawn(async move |cx| {
        serve(stream, checker, handler, token.as_deref(), cx)
            .await
            .ok()
    })
    .detach();
}

/// One request per connection: a JSON line in, `{"text"}` or `{"error"}` out.
async fn serve<S>(
    mut stream: S,
    checker: WeakEntity<Checker>,
    handler: Handler,
    token: Option<&str>,
    cx: &mut AsyncApp,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Clone + Unpin,
{
    use smol::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _};
    let mut line = String::new();
    smol::io::BufReader::new(stream.clone().take(64 * 1024))
        .read_line(&mut line)
        .await?;
    let answer = async {
        let mut request = serde_json::from_str::<serde_json::Value>(&line)?;
        let sent = request
            .as_object_mut()
            .and_then(|request| request.remove("token"));
        if let Some(token) = token
            && sent.as_ref().and_then(|sent| sent.as_str()) != Some(token)
        {
            anyhow::bail!("Missing or wrong token");
        }
        if !matches!(request["op"].as_str(), Some("file" | "run_end")) {
            return cx.update(|cx| handler(request, cx)).await;
        }
        let request = serde_json::from_value::<Request>(request)?;
        checker.update(cx, |c, cx| c.handle(request, cx))?.await
    }
    .await;
    let reply = match answer {
        Ok(text) => serde_json::json!({ "text": text }),
        Err(error) => serde_json::json!({ "error": format!("{error:#}") }),
    };
    stream.write_all(format!("{reply}\n").as_bytes()).await?;
    Ok(())
}
