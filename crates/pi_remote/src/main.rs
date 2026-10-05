//! Headless SSH helper. The stdio bridge is disposable; the detached daemon owns Pi.
mod deletion;
mod directories;
mod durable;
mod executable;
mod files;
mod server;
mod sessions;
use anyhow::{Context, Result, bail};
use pi_core::ssh::{PROTOCOL_VERSION, VERSION};

fn platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-amd64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "aarch64") => "macos-arm64",
        ("windows", "x86_64") => "windows-amd64",
        _ => "unsupported",
    }
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("--version") => println!(
            "pi-desktop-remote {VERSION} {PROTOCOL_VERSION} {}",
            platform()
        ),
        Some("--capabilities") => println!(
            "{}",
            serde_json::json!({"pi":true,"durable":cfg!(unix),"durableExperimental":true,"watchers":true,"sessions":true,"directories":true,"deleteSessions":cfg!(unix),"imagePrompts":cfg!(feature = "bundled-durable")})
        ),
        Some("--licenses") => {
            println!(
                "{}\n{}\n{}\n{}",
                include_str!("../../../LICENSE"),
                include_str!("../../../THIRD_PARTY.md"),
                include_str!("../../../licenses/PI-MIT.txt"),
                include_str!("../../../licenses/BUN-LICENSE.md")
            );
            #[cfg(feature = "bundled-backend")]
            println!("{}", include_str!(env!("PI_DESKTOP_BACKEND_NOTICES")));
            #[cfg(feature = "bundled-durable")]
            println!("{}", include_str!(env!("PI_DESKTOP_DURABLE_NOTICES")));
        }
        Some("durable-worker") => {
            let target =
                serde_json::from_str(&args.next().context("durable-worker needs a target")?)?;
            durable::worker(target)?;
        }
        Some("pi") => {
            // Interactive setup/login uses the same bundled Pi, with the SSH user's own credentials.
            let backend = backend()?;
            let program = std::env::var_os("PI_DESKTOP_PI")
                .or_else(|| backend.program.map(Into::into))
                .unwrap_or_else(|| if cfg!(windows) { "pi.cmd" } else { "pi" }.into());
            let mut command = std::process::Command::new(program);
            command.args(args);
            if std::env::var_os("NODE_USE_SYSTEM_CA").is_none() {
                command.env("NODE_USE_SYSTEM_CA", "1");
            }
            let status = command
                .status()
                .context("Could not launch Pi for interactive setup")?;
            std::process::exit(status.code().unwrap_or(1));
        }
        Some("connect") if args.next().as_deref() == Some("--stdio") => server::connect()?,
        Some("sessions") => sessions::print()?,
        Some("models") => {
            anyhow::ensure!(args.next().is_none(), "Unexpected models argument");
            durable::models()?;
        }
        Some("directories") if args.next().as_deref() == Some("--path") => {
            let path = args.next().context("directories needs a path")?;
            let show_hidden = match args.next().as_deref() {
                None => false,
                Some("--show-hidden") => true,
                _ => bail!("Unknown directories option"),
            };
            anyhow::ensure!(args.next().is_none(), "Unexpected directories argument");
            directories::print(&path, show_hidden)?;
        }
        Some("files") if args.next().as_deref() == Some("--stdio") => files::serve()?,
        Some("daemon") => {
            let target = serde_json::from_str(&args.next().context("daemon needs a target")?)?;
            server::daemon(target)?;
        }
        _ => bail!(
            "Usage: pi-desktop-remote connect --stdio | files --stdio | sessions | models | directories --path PATH [--show-hidden] | pi [ARGS] | --version | --licenses"
        ),
    }
    Ok(())
}

fn backend() -> Result<pi_core::transport::Backend> {
    #[cfg(feature = "bundled-backend")]
    {
        // Never remove an older backend: an existing daemon may still use its resources.
        static ARCHIVE: &[u8] = include_bytes!(env!("PI_DESKTOP_BACKEND_ARCHIVE"));
        let root = dirs::cache_dir()
            .context("No cache directory")?
            .join("pi-desktop-remote/backend")
            .join(env!("PI_DESKTOP_BACKEND_ID"));
        let program = root.join(if cfg!(windows) { "pi.exe" } else { "pi" });
        if !program.is_file() {
            std::fs::create_dir_all(root.parent().context("backend parent")?)?;
            let staging = tempfile::tempdir_in(root.parent().context("backend parent")?)?;
            tar::Archive::new(flate2::read::GzDecoder::new(ARCHIVE)).unpack(staging.path())?;
            anyhow::ensure!(
                staging
                    .path()
                    .join(if cfg!(windows) { "pi.exe" } else { "pi" })
                    .is_file(),
                "Archive has no Pi binary"
            );
            if let Err(error) = std::fs::rename(staging.path(), &root) {
                anyhow::ensure!(program.is_file(), "Could not install bundled Pi: {error}");
            }
        }
        Ok(pi_core::transport::Backend {
            program: Some(program),
        })
    }
    #[cfg(not(feature = "bundled-backend"))]
    {
        Ok(pi_core::transport::Backend::default())
    }
}
