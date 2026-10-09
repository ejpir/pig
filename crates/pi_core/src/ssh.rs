//! OpenSSH transport and per-user, checksum-verified remote helper installation.
//! Only fixed installation commands cross a shell. Project paths travel as JSON.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{bounded_output, transport::Launch};

pub const PROTOCOL_VERSION: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Backend choice is persisted with a session identity, never changed by reconnect.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteBackend {
    #[default]
    Pi,
    Durable,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshTarget {
    pub host: String,
    pub cwd: String,
    /// A stable random identity, independent of Pi's session file and the SSH connection.
    pub key: String,
    #[serde(default)]
    pub backend: RemoteBackend,
    #[serde(default)]
    pub session_file: Option<String>,
}
impl SshTarget {
    pub fn new(host: String, cwd: String) -> Result<Self> {
        let target = Self {
            host,
            cwd,
            key: format!("{:032x}", rand::random::<u128>()),
            backend: RemoteBackend::Pi,
            session_file: None,
        };
        target.validate()?;
        Ok(target)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.backend != RemoteBackend::Durable || self.session_file.is_none(),
            "Durable sessions cannot open or migrate a stock Pi session file"
        );
        ensure!(
            !self.host.is_empty()
                && !self.host.starts_with('-')
                && self.host.len() <= 255
                && self
                    .host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-@[]:".contains(&b)),
            "Use an SSH config host alias or user@hostname (no options or whitespace)"
        );
        ensure!(
            self.key.len() == 32 && self.key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid remote session key"
        );
        ensure!(
            !self.cwd.is_empty()
                && self.cwd.len() <= 4096
                && !self.cwd.contains(['\0', '\n', '\r']),
            "Choose a remote project directory"
        );
        Ok(())
    }
    /// Presentation only. This path must never be passed to a local filesystem service.
    pub fn identity(&self) -> PathBuf {
        PathBuf::from(format!(
            "ssh://{}/{}",
            self.host,
            self.cwd.trim_start_matches('/')
        ))
    }
    pub fn attach_record(&self) -> serde_json::Value {
        serde_json::json!({"type":"remote_attach", "version":PROTOCOL_VERSION, "target":self})
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    LinuxX64,
    LinuxArm64,
    MacArm64,
    WindowsX64,
}
impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Self::LinuxX64 => "linux-amd64",
            Self::LinuxArm64 => "linux-arm64",
            Self::MacArm64 => "macos-arm64",
            Self::WindowsX64 => "windows-amd64",
        }
    }
    pub fn asset(self) -> String {
        format!(
            "pi-desktop-remote-{}{}",
            self.name(),
            if self == Self::WindowsX64 { ".exe" } else { "" }
        )
    }
    fn executable(self) -> &'static str {
        if self == Self::WindowsX64 {
            "pi-desktop-remote.exe"
        } else {
            "pi-desktop-remote"
        }
    }
}

fn ssh(host: &str) -> ProcessCommand {
    let mut command = ProcessCommand::new("ssh");
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        "ConnectTimeout=15",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        "--",
        host,
    ]);
    command
}
fn checked(command: &mut ProcessCommand) -> Result<std::process::Output> {
    let output = bounded_output(command, Duration::from_secs(180))?;
    ensure!(
        output.status.success(),
        "Remote setup failed: {}. Configure keys/agent and verify this host with ssh in a terminal first.",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output)
}
fn detect(host: &str) -> Result<Platform> {
    let output = bounded_output(ssh(host).arg("uname -s; uname -m"), Duration::from_secs(30))?;
    if output.status.success() {
        return match String::from_utf8_lossy(&output.stdout).trim() {
            "Linux\nx86_64" => Ok(Platform::LinuxX64),
            "Linux\naarch64" | "Linux\narm64" => Ok(Platform::LinuxArm64),
            "Darwin\narm64" => Ok(Platform::MacArm64),
            other => bail!("Unsupported remote platform: {other}"),
        };
    }
    let output = checked(ssh(host).arg("powershell -NoProfile -NonInteractive -Command \"[System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()\""))?;
    ensure!(
        String::from_utf8_lossy(&output.stdout).trim() == "X64",
        "Only x64 Windows SSH hosts are currently packaged"
    );
    Ok(Platform::WindowsX64)
}
fn digest(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn download(url: &str, destination: &Path) -> Result<()> {
    let output = bounded_output(
        ProcessCommand::new("curl")
            .args([
                "--fail-with-body",
                "--location",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--proto-redir",
                "=https",
                "--max-time",
                "180",
                "--max-filesize",
                "536870912",
                "--output",
            ])
            .arg(destination)
            .arg(url),
        Duration::from_secs(190),
    )?;
    if !output.status.success() {
        use std::io::Read as _;
        let mut body = Vec::new();
        if let Ok(file) = fs::File::open(destination) {
            let _ = file.take(8192).read_to_end(&mut body);
        }
        bail!(
            "Could not download remote helper: {}\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&body)
        );
    }
    Ok(())
}
fn helper(platform: Platform) -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("PI_DESKTOP_REMOTE_HELPER") {
        let path = PathBuf::from(path).canonicalize()?;
        validate_binary(&path, platform)?;
        return Ok(path);
    }
    let repository = option_env!("PI_DESKTOP_RELEASE_REPOSITORY").unwrap_or("ejpir/pig");
    let root = dirs::cache_dir()
        .context("No cache directory")?
        .join("pi-desktop/remote")
        .join(format!("{:x}", Sha256::digest(repository.as_bytes())))
        .join(VERSION);
    fs::create_dir_all(&root)?;
    if let Some(path) = cached_helper(&root, platform) {
        return Ok(path);
    }
    let base = format!("https://github.com/{repository}/releases/download/v{VERSION}");
    let staging = tempfile::tempdir_in(&root)?;
    let sums = staging.path().join("SHA256SUMS");
    download(&format!("{base}/SHA256SUMS"), &sums)?;
    let asset = platform.asset();
    ensure!(
        fs::metadata(&sums)?.len() <= 1024 * 1024,
        "Helper checksum manifest is too large"
    );
    let manifest = fs::read_to_string(&sums)?;
    let expected = checksum(&manifest, &asset)?;
    let installed = root.join(&asset);
    if !installed.is_file() || digest(&installed)? != expected {
        let partial = tempfile::NamedTempFile::new_in(&root)?;
        download(&format!("{base}/{asset}"), partial.path())?;
        ensure!(
            digest(partial.path())? == expected,
            "Remote helper checksum mismatch"
        );
        validate_binary(partial.path(), platform)?;
        partial.persist(&installed)?;
    }
    // Releases are immutable. A previously verified cache can reconnect without GitHub access.
    use std::io::Write as _;
    let mut saved_manifest = tempfile::NamedTempFile::new_in(&root)?;
    saved_manifest.write_all(manifest.as_bytes())?;
    saved_manifest.persist(root.join("SHA256SUMS"))?;
    Ok(installed)
}
fn cached_helper(root: &Path, platform: Platform) -> Option<PathBuf> {
    let sums = root.join("SHA256SUMS");
    if fs::metadata(&sums).ok()?.len() > 1024 * 1024 {
        return None;
    }
    let manifest = fs::read_to_string(sums).ok()?;
    let path = root.join(platform.asset());
    let expected = checksum(&manifest, &platform.asset()).ok()?;
    if digest(&path).ok()? != expected || validate_binary(&path, platform).is_err() {
        return None;
    }
    Some(path)
}
fn checksum(manifest: &str, asset: &str) -> Result<String> {
    let matches: Vec<_> = manifest
        .lines()
        .filter_map(|line| line.split_once("  "))
        .filter(|(_, name)| *name == asset)
        .collect();
    ensure!(
        matches.len() == 1,
        "Release has no unique checksum for {asset}"
    );
    let hash = matches[0].0;
    ensure!(
        hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid helper checksum"
    );
    Ok(hash.to_ascii_lowercase())
}
fn validate_binary(path: &Path, platform: Platform) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = fs::File::open(path)?;
    let mut header = [0; 64];
    file.read_exact(&mut header)?;
    let valid = match platform {
        Platform::LinuxX64 | Platform::LinuxArm64 => {
            header[..6] == *b"\x7fELF\x02\x01"
                && u16::from_le_bytes([header[18], header[19]])
                    == if platform == Platform::LinuxX64 {
                        62
                    } else {
                        183
                    }
        }
        Platform::MacArm64 => header[..8] == [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1],
        Platform::WindowsX64 => {
            file.seek(SeekFrom::Start(
                u32::from_le_bytes(header[60..64].try_into()?) as u64,
            ))?;
            let mut pe = [0; 6];
            header[..2] == *b"MZ" && file.read_exact(&mut pe).is_ok() && pe == *b"PE\0\0\x64\x86"
        }
    };
    ensure!(valid, "Helper is not a native {} binary", platform.name());
    Ok(())
}
fn shell_script(script: &str) -> String {
    // The account's login shell may be fish. Explicitly run our fixed POSIX script in sh.
    format!("sh -c '{}'", script.replace('\'', "'\"'\"'"))
}
fn remote_command(platform: Platform, hash: &str, arguments: &str) -> String {
    let executable = platform.executable();
    if platform == Platform::WindowsX64 {
        format!(
            "powershell -NoProfile -NonInteractive -Command \"& ($env:USERPROFILE + '/.pi/desktop/bin/{hash}/{executable}') {arguments}; exit $LASTEXITCODE\""
        )
    } else {
        shell_script(&format!(
            "exec \"$HOME/.pi/desktop/bin/{hash}/{executable}\" {arguments}"
        ))
    }
}

fn probe_command(platform: Platform, hash: &str) -> String {
    if platform == Platform::WindowsX64 {
        return remote_command(platform, hash, "--version");
    }
    let executable = platform.executable();
    shell_script(&format!(
        "set -eu; p=\"$HOME/.pi/desktop/bin/{hash}/{executable}\"; test -f \"$p\"; test ! -L \"$p\"; actual=$( (sha256sum \"$p\" 2>/dev/null || shasum -a 256 \"$p\") | cut -d ' ' -f 1); test \"$actual\" = '{hash}'; exec \"$p\" --version"
    ))
}

fn unix_install_object_command(
    directory: &str,
    temporary: &str,
    executable: &str,
    hash: &str,
) -> String {
    format!(
        "set -eu; cd \"$HOME/{directory}\"; trap 'rm -f {temporary}' EXIT; test -f '{temporary}'; test ! -L '{temporary}'; actual=$( (sha256sum '{temporary}' 2>/dev/null || shasum -a 256 '{temporary}') | cut -d ' ' -f 1); test \"$actual\" = '{hash}'; chmod 700 '{temporary}'; if test -e '{executable}' || test -L '{executable}'; then test -f '{executable}'; test ! -L '{executable}'; existing=$( (sha256sum '{executable}' 2>/dev/null || shasum -a 256 '{executable}') | cut -d ' ' -f 1); test \"$existing\" = '{hash}'; else ln '{temporary}' '{executable}'; fi; rm -f '{temporary}'; trap - EXIT"
    )
}

fn activation_command(platform: Platform, hash: &str) -> Option<String> {
    (platform != Platform::WindowsX64).then(|| remote_command(platform, hash, "activate"))
}

/// Blocking bootstrap; call on a background executor. No remote cwd is interpolated into a shell.
pub fn install(target: &SshTarget) -> Result<Launch> {
    install_mode(target, "connect --stdio")
}

/// Independent file channel; never starts Pi or attaches to its control stream.
pub fn install_files(target: &SshTarget) -> Result<Launch> {
    install_mode(target, "files --stdio")
}

fn install_mode(target: &SshTarget, mode: &str) -> Result<Launch> {
    target.validate()?;
    let platform = detect(&target.host)?;
    let helper = helper(platform)?;
    let hash = digest(&helper)?;
    let executable = platform.executable();
    let probe = bounded_output(
        ssh(&target.host).arg(probe_command(platform, &hash)),
        Duration::from_secs(30),
    )?;
    let expected = format!(
        "pi-desktop-remote {VERSION} {PROTOCOL_VERSION} {}",
        platform.name()
    );
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != expected {
        let directory = format!(".pi/desktop/bin/{hash}");
        let temporary = format!("{executable}.{:08x}.partial", rand::random::<u32>());
        if platform == Platform::WindowsX64 {
            checked(ssh(&target.host).arg(format!("powershell -NoProfile -NonInteractive -Command \"New-Item -ItemType Directory -Force ($env:USERPROFILE + '/{directory}') | Out-Null\"")))?;
        } else {
            checked(ssh(&target.host).arg(shell_script(&format!("set -eu; umask 077; mkdir -p \"$HOME/{directory}\"; chmod 700 \"$HOME/.pi\" \"$HOME/.pi/desktop\" \"$HOME/.pi/desktop/bin\" \"$HOME/{directory}\""))))?;
        }
        checked(
            ProcessCommand::new("scp")
                .args([
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "StrictHostKeyChecking=yes",
                    "-o",
                    "ConnectTimeout=15",
                    "--",
                ])
                .arg(&helper)
                .arg(format!("{}:{directory}/{temporary}", target.host)),
        )?;
        let command = if platform == Platform::WindowsX64 {
            format!(
                "powershell -NoProfile -NonInteractive -Command \"$p=$env:USERPROFILE+'/{directory}/{temporary}'; if ((Get-FileHash -Algorithm SHA256 $p).Hash.ToLower() -ne '{hash}') {{ throw 'Helper checksum mismatch' }}; Move-Item -Force $p ($env:USERPROFILE+'/{directory}/{executable}')\""
            )
        } else {
            unix_install_object_command(&directory, &temporary, executable, &hash)
        };
        let command = if platform == Platform::WindowsX64 {
            command
        } else {
            shell_script(&command)
        };
        checked(ssh(&target.host).arg(command))?;
        let verified = checked(ssh(&target.host).arg(probe_command(platform, &hash)))?;
        ensure!(
            String::from_utf8_lossy(&verified.stdout).trim() == expected,
            "Remote helper version mismatch"
        );
    }
    if let Some(command) = activation_command(platform, &hash) {
        checked(ssh(&target.host).arg(command))?;
    }
    if mode == "connect --stdio" && target.backend == RemoteBackend::Durable {
        let capabilities = checked(ssh(&target.host).arg(remote_command(
            platform,
            &hash,
            "--capabilities",
        )))
        .context(
            "This helper does not support durable sessions; rebuild it with the durable prototype",
        )?;
        let capabilities: serde_json::Value = serde_json::from_slice(&capabilities.stdout)?;
        ensure!(
            capabilities["durable"] == true,
            "The durable prototype requires a supported Unix SSH helper"
        );
    }
    let command = ssh(&target.host);
    let mut args: Vec<_> = command.get_args().map(|arg| arg.to_owned()).collect();
    args.push(remote_command(platform, &hash, mode).into());
    Ok(Launch {
        program: "ssh".into(),
        args,
        cwd: std::env::current_dir()?,
        env: Vec::new(),
        request_timeout: Duration::from_secs(30),
        extension: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_option_and_shell_injection_hosts() {
        for host in [
            "-oProxyCommand=evil",
            "host;evil",
            "host\n",
            "host name",
            "$(evil)",
        ] {
            assert!(SshTarget::new(host.into(), "/work".into()).is_err());
        }
        for host in ["dev", "user@dev.example", "user@[::1]"] {
            assert!(SshTarget::new(host.into(), "/work/a 'quoted' repo".into()).is_ok());
        }
    }
    #[test]
    fn targets_round_trip_without_treating_remote_paths_as_local() {
        let target = SshTarget::new("dev".into(), "C:\\repos\\test".into()).unwrap();
        assert_eq!(
            serde_json::from_value::<SshTarget>(serde_json::to_value(&target).unwrap()).unwrap(),
            target
        );
        assert!(target.identity().to_string_lossy().starts_with("ssh:"));
        assert_eq!(target.attach_record()["target"]["cwd"], target.cwd);
    }
    #[test]
    fn verified_cached_helpers_need_no_network_and_corruption_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let platform = Platform::LinuxArm64;
        let path = root.path().join(platform.asset());
        let mut bytes = vec![0; 64];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[18] = 183;
        fs::write(&path, &bytes).unwrap();
        assert!(cached_helper(root.path(), platform).is_none());
        fs::write(
            root.path().join("SHA256SUMS"),
            format!("{}  {}\n", digest(&path).unwrap(), platform.asset()),
        )
        .unwrap();
        assert_eq!(cached_helper(root.path(), platform), Some(path.clone()));
        fs::write(&path, b"corrupted").unwrap();
        assert!(cached_helper(root.path(), platform).is_none());
    }
    #[test]
    fn checksums_are_exact_and_unique() {
        let hash = "a".repeat(64);
        assert_eq!(
            checksum(&format!("{hash}  helper\n"), "helper").unwrap(),
            hash
        );
        assert!(checksum(&format!("{hash}  helper\n{hash}  helper\n"), "helper").is_err());
        assert!(checksum("abc  helper", "helper").is_err());
        assert!(checksum(&format!("{hash}  helper-other"), "helper").is_err());
    }
    #[test]
    fn installation_shell_commands_verify_immutable_objects_and_activate_unix() {
        let hash = "a".repeat(64);
        let command = remote_command(Platform::LinuxX64, &hash, "connect --stdio");
        assert!(command.starts_with("sh -c "));
        assert!(command.contains("exec "));
        assert!(!command.contains("--cwd"));
        let probe = probe_command(Platform::LinuxX64, &hash);
        assert!(probe.contains("sha256sum"));
        assert!(probe.contains(&hash));
        let install = unix_install_object_command(
            ".pi/desktop/bin/hash",
            "helper.partial",
            "pi-desktop-remote",
            &hash,
        );
        assert!(install.contains("test ! -L 'helper.partial'"));
        assert!(install.contains("test ! -L 'pi-desktop-remote'"));
        assert!(install.contains("ln 'helper.partial' 'pi-desktop-remote'"));
        assert!(!install.contains("mv -f 'helper.partial'"));
        let activation = activation_command(Platform::LinuxX64, &hash).unwrap();
        assert!(activation.contains(&format!(
            "$HOME/.pi/desktop/bin/{hash}/pi-desktop-remote\" activate"
        )));
        assert_eq!(activation_command(Platform::WindowsX64, &hash), None);
        assert!(
            remote_command(Platform::WindowsX64, &hash, "connect --stdio").contains("USERPROFILE")
        );
    }
}
