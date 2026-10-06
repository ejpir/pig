//! One-use QR enrollment through the computer's existing OpenSSH server.
//! No Pi TCP service listens: a restricted bootstrap key can only run `pair exchange`.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use fs2::FileExt as _;
use pi_core::pairing::{self, Helper, Offer, Request, Response, VERSION, confirmation_code};
use qrcode::{QrCode, render::unicode};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use ssh_key::{Algorithm, HashAlg, PrivateKey, PublicKey, private::Ed25519Keypair};
use std::{
    collections::BTreeSet,
    env,
    fs::{self, OpenOptions},
    io::{self, BufRead as _, Read as _, Write as _},
    net::UdpSocket,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

const DEFAULT_LIFETIME: u64 = 120;
const MAX_REQUEST: u64 = 16 * 1024;
const BOOTSTRAP_COMMENT: &str = "pi-pair-bootstrap:";
const PHONE_COMMENT: &str = "pi-phone:";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    id: String,
    expires_at: u64,
    bootstrap_marker: String,
}

/// Once the bootstrap key is installed, every return path removes it and its
/// short-lived exchange files. A crash can still leave it behind, but the
/// forced exchange checks the expired state and the next pairing cleans it.
struct BootstrapGuard(State);

impl Drop for BootstrapGuard {
    fn drop(&mut self) {
        cleanup(&self.0).ok();
    }
}

#[derive(Default)]
struct Options {
    hosts: Vec<String>,
    port: u16,
    lifetime: u64,
    json: bool,
    yes: bool,
}

impl Options {
    fn parse(arguments: &[String]) -> Result<Self> {
        let mut options = Self {
            port: 22,
            lifetime: DEFAULT_LIFETIME,
            ..Self::default()
        };
        let mut arguments = arguments.iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--host" => options
                    .hosts
                    .push(arguments.next().context("--host needs an address")?.clone()),
                "--port" => {
                    options.port = arguments
                        .next()
                        .context("--port needs a number")?
                        .parse()
                        .context("--port needs a number from 1 to 65535")?;
                }
                "--expires" => {
                    options.lifetime = arguments
                        .next()
                        .context("--expires needs seconds")?
                        .parse()
                        .context("--expires needs a number of seconds")?;
                }
                "--json" => options.json = true,
                "--yes" => options.yes = true,
                _ => bail!(
                    "Usage: pi-desktop-remote pair [--host HOST] [--port PORT] [--expires SECONDS] [--json] [--yes]"
                ),
            }
        }
        if options.port == 0 || !(30..=600).contains(&options.lifetime) {
            bail!("Pairing expiry must be between 30 and 600 seconds");
        }
        Ok(options)
    }
}

pub fn pair(arguments: &[String]) -> Result<()> {
    if !cfg!(unix) {
        bail!("QR pairing currently requires OpenSSH on macOS or Linux");
    }
    let mut options = Options::parse(arguments)?;
    cleanup_expired()?;
    if options.hosts.is_empty() {
        options.hosts = local_hosts();
    }
    if options.hosts.is_empty() {
        bail!("No computer address was found; pass --host with an address the phone can reach");
    }
    let mut seen = BTreeSet::new();
    options.hosts.retain(|host| seen.insert(host.clone()));
    let fingerprints = host_keys(options.port)?;

    let mut seed = [0u8; 32];
    rand::rng().fill_bytes(&mut seed);
    let id = random_id();
    let mut bootstrap = PrivateKey::from(Ed25519Keypair::from_seed(&seed));
    bootstrap.set_comment(format!("{BOOTSTRAP_COMMENT}{id}"));
    let bootstrap_public = bootstrap.public_key().to_openssh()?;
    let marker = format!("{BOOTSTRAP_COMMENT}{id}");
    let state = State {
        id: id.clone(),
        expires_at: pairing::now() + options.lifetime,
        bootstrap_marker: marker.clone(),
    };
    let helper = helper(true)?;
    let forced = forced_command(&helper.path, &["pair", "exchange", &id])?;
    let line = format!("restrict,command={forced} {bootstrap_public}\n");
    edit_authorized_keys(&[&marker], None, Some(&line))?;
    let _bootstrap = BootstrapGuard(state.clone());
    write_json(&state_path(&id)?, &state)?;

    let offer = Offer {
        version: VERSION,
        id: id.clone(),
        user: whoami::username(),
        hosts: options.hosts,
        port: options.port,
        host_keys: fingerprints,
        bootstrap_seed: URL_SAFE_NO_PAD.encode(seed),
        expires_at: state.expires_at,
    };
    let url = offer.url()?;
    if options.json {
        println!("{url}");
    } else {
        println!("\nPair this computer with Pi on Android\n");
        println!("{}", terminal_qr(&url)?);
        println!(
            "Open Pi → Scan computer. This code expires in {} seconds.\n",
            options.lifetime
        );
    }
    io::stdout().flush()?;

    let request_path = request_path(&id)?;
    let deadline = Instant::now() + Duration::from_secs(options.lifetime);
    let request: Request = loop {
        if Instant::now() >= deadline {
            cleanup(&state)?;
            bail!("Pairing timed out; no key was authorized");
        }
        match read_json(&request_path) {
            Ok(request) => break request,
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
            {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                cleanup(&state)?;
                return Err(error.context("Could not read the phone's pairing request"));
            }
        }
    };
    let phone_key = validate_request(&state, &request)?;
    let code = confirmation_code(&id, &request.public_key);
    println!("{} requests access.", clean_name(&request.device_name));
    println!("Confirmation code: {code}");
    let approved = if options.yes {
        true
    } else {
        print!("Press Enter to allow, or type n to deny: ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        !matches!(answer.trim().to_ascii_lowercase().as_str(), "n" | "no")
    };

    let response = if approved {
        let (permanent, phone_marker) = permanent_line(&helper.path, phone_key.clone())?;
        // Re-pairing the same phone upgrades its forced command to this helper
        // instead of leaving an older matching key earlier in the file. Match
        // the key material too: a key copied during manual setup has a different
        // comment and no Pi marker, but OpenSSH would otherwise accept that
        // unrestricted entry before reaching the gateway entry below it.
        edit_authorized_keys(
            &[&marker, &phone_marker],
            Some(&phone_key),
            Some(&permanent),
        )?;
        Response {
            version: VERSION,
            approved: true,
            error: None,
            helper: Some(helper),
        }
    } else {
        edit_authorized_keys(&[&marker], None, None)?;
        Response {
            version: VERSION,
            approved: false,
            error: Some("Pairing was denied on the computer".into()),
            helper: None,
        }
    };
    write_json(&response_path(&id)?, &response)?;
    wait_for_done(&id, Duration::from_secs(10))?;
    cleanup_files(&id);
    if approved {
        println!("Paired. The phone now has restricted access to Pi's helper.");
        Ok(())
    } else {
        bail!("Pairing denied; no phone key was authorized")
    }
}

/// Runs inside the one-use forced SSH command. It cannot authorize anything itself;
/// the foreground `pair` process confirms and edits the key file.
pub fn exchange(id: &str) -> Result<()> {
    valid_id(id)?;
    let marker = format!("{BOOTSTRAP_COMMENT}{id}");
    let state: State = match state_path(id).and_then(|path| read_json(&path)) {
        Ok(state) => state,
        Err(error) => {
            // A killed foreground pairing process may have installed the key
            // just before its state write. Its only usable command removes it.
            edit_authorized_keys(&[&marker], None, None).ok();
            cleanup_files(id);
            return Err(error).context("This pairing code is no longer active");
        }
    };
    if state.id != id || pairing::now() > state.expires_at {
        edit_authorized_keys(&[&marker], None, None).ok();
        cleanup_files(id);
        bail!("This pairing code expired");
    }
    let mut input = String::new();
    io::stdin()
        .lock()
        .take(MAX_REQUEST)
        .read_line(&mut input)
        .context("Could not read the phone's pairing request")?;
    let request: Request = serde_json::from_str(input.trim()).context("Invalid pairing request")?;
    validate_request(&state, &request)?;
    write_new_json(&request_path(id)?, &request)?;

    let deadline = Instant::now()
        + Duration::from_secs(state.expires_at.saturating_sub(pairing::now()).min(600));
    let response: Response = loop {
        if Instant::now() >= deadline {
            bail!("Pairing approval timed out");
        }
        match response_path(id).and_then(|path| read_json(&path)) {
            Ok(response) => break response,
            Err(error)
                if error
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::NotFound) =>
            {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => return Err(error.context("Could not read pairing approval")),
        }
    };
    println!("{}", serde_json::to_string(&response)?);
    write_private(&done_path(id)?, b"done\n", true)?;
    Ok(())
}

pub fn helper(gateway: bool) -> Result<Helper> {
    let executable = env::current_exe()?
        .canonicalize()
        .context("The running helper cannot be used for a lasting pairing")?;
    Ok(Helper {
        path: executable.to_string_lossy().into_owned(),
        home: dirs::home_dir()
            .context("This account has no home folder")?
            .to_string_lossy()
            .trim_end_matches('/')
            .to_owned(),
        images: cfg!(feature = "bundled-durable"),
        gateway,
    })
}

pub fn print_discovery(gateway: bool) -> Result<()> {
    println!("{}", serde_json::to_string(&helper(gateway)?)?);
    Ok(())
}

/// Returns the original app command accepted by a paired phone's forced command.
pub fn gateway_command() -> Result<Vec<String>> {
    let original = env::var("SSH_ORIGINAL_COMMAND").context("The SSH gateway needs a command")?;
    let mut command = shell_words::split(&original).context("Invalid SSH command")?;
    if command.first().map(String::as_str) != Some("pi-desktop-remote") {
        bail!("This phone key can only run Pi's remote helper");
    }
    command.remove(0);
    let allowed = matches!(command.as_slice(),
        [one] if matches!(one.as_str(), "sessions" | "models" | "commands" | "discover" | "--version" | "--capabilities")
    ) || matches!(command.as_slice(), [one, two] if
        (one == "connect" || one == "files") && two == "--stdio"
    ) || matches!(command.as_slice(), [one, two, _] if one == "directories" && two == "--path")
        || matches!(command.as_slice(), [one, two, _] if matches!(one.as_str(), "jj-history" | "jj-enable") && two == "--path")
        || matches!(command.as_slice(), [one, two, _, three, _] if one == "jj-restore" && two == "--path" && three == "--operation")
        || matches!(command.as_slice(), [one, two, _, three] if one == "directories" && two == "--path" && three == "--show-hidden");
    if !allowed {
        bail!("This phone key cannot run that command");
    }
    Ok(command)
}

fn validate_request(state: &State, request: &Request) -> Result<PublicKey> {
    if request.version != VERSION || request.id != state.id {
        bail!("The pairing request doesn't match this code");
    }
    if request.device_name.len() > 128 || request.device_name.chars().any(char::is_control) {
        bail!("The phone name is invalid");
    }
    let key = PublicKey::from_openssh(&request.public_key).context("The phone key is invalid")?;
    if key.algorithm() != Algorithm::Ed25519 {
        bail!("The phone must use an Ed25519 key");
    }
    Ok(key)
}

fn permanent_line(path: &str, mut key: PublicKey) -> Result<(String, String)> {
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let short: String = fingerprint
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect();
    let device = format!("{PHONE_COMMENT}{short}");
    key.set_comment(device.as_str());
    let forced = forced_command(path, &["gateway", &device])?;
    Ok((
        format!("restrict,command={forced} {}\n", key.to_openssh()?),
        device,
    ))
}

fn forced_command(program: &str, arguments: &[&str]) -> Result<String> {
    if program.contains(['\n', '\r', '\0'])
        || arguments.iter().any(|arg| arg.contains(['\n', '\r', '\0']))
    {
        bail!("The helper path cannot be represented safely in authorized_keys");
    }
    let mut shell = format!("exec {}", shell_words::quote(program));
    for argument in arguments {
        shell.push(' ');
        shell.push_str(&shell_words::quote(argument));
    }
    // authorized_keys has its own double-quoted option syntax around the shell string.
    let escaped = shell.replace('\\', "\\\\").replace('"', "\\\"");
    Ok(format!("\"{escaped}\""))
}

fn host_keys(port: u16) -> Result<Vec<String>> {
    let output = Command::new("ssh-keyscan")
        .args(["-T", "5", "-p", &port.to_string(), "127.0.0.1"])
        .output()
        .context("ssh-keyscan is required to verify this computer")?;
    let mut fingerprints = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut fields = line.split_whitespace();
        let (Some(_host), Some(algorithm), Some(data)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        if let Ok(key) = PublicKey::from_openssh(&format!("{algorithm} {data}")) {
            fingerprints.insert(key.fingerprint(HashAlg::Sha256).to_string());
        }
    }
    if fingerprints.is_empty() {
        bail!("The SSH server on port {port} did not answer. Enable Remote Login before pairing.");
    }
    Ok(fingerprints.into_iter().collect())
}

/// Unicode block glyphs normally inherit the terminal theme, which reverses
/// the QR on a dark terminal. Explicit ANSI foreground/background colors keep
/// the machine-readable symbol black on white on either theme.
fn terminal_qr(value: &str) -> Result<String> {
    let image = QrCode::new(value.as_bytes())?
        .render::<unicode::Dense1x2>()
        .quiet_zone(true)
        .build();
    Ok(format!("\x1b[30;47m{image}\x1b[0m"))
}

fn local_hosts() -> Vec<String> {
    let mut hosts = Vec::new();
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0")
        && socket.connect("192.0.2.1:9").is_ok()
        && let Ok(address) = socket.local_addr()
        && !address.ip().is_loopback()
    {
        hosts.push(address.ip().to_string());
    }
    if let Ok(output) = Command::new("hostname").output()
        && output.status.success()
    {
        let host = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !host.is_empty() && !host.chars().any(char::is_whitespace) {
            if !host.contains('.') {
                hosts.push(format!("{host}.local"));
            }
            hosts.push(host);
        }
    }
    hosts
}

fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_id(id: &str) -> Result<()> {
    if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("Invalid pairing identity");
    }
    Ok(())
}

fn clean_name(name: &str) -> &str {
    if name.trim().is_empty() {
        "An Android phone"
    } else {
        name.trim()
    }
}

fn pairing_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("PI_DESKTOP_PAIRING_DIR") {
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::home_dir()
        .context("This account has no home folder")?
        .join(".pi/desktop/pairing"))
}

fn state_path(id: &str) -> Result<PathBuf> {
    Ok(pairing_dir()?.join(format!("{id}.state.json")))
}
fn request_path(id: &str) -> Result<PathBuf> {
    Ok(pairing_dir()?.join(format!("{id}.request.json")))
}
fn response_path(id: &str) -> Result<PathBuf> {
    Ok(pairing_dir()?.join(format!("{id}.response.json")))
}
fn done_path(id: &str) -> Result<PathBuf> {
    Ok(pairing_dir()?.join(format!("{id}.done")))
}

fn authorized_keys_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("PI_DESKTOP_AUTHORIZED_KEYS_FILE") {
        return Ok(PathBuf::from(path));
    }
    Ok(dirs::home_dir()
        .context("This account has no home folder")?
        .join(".ssh/authorized_keys"))
}

fn edit_authorized_keys(
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
    append: Option<&str>,
) -> Result<()> {
    edit_authorized_keys_at(authorized_keys_path()?, remove_markers, remove_key, append)
}

fn edit_authorized_keys_at(
    requested: PathBuf,
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
    append: Option<&str>,
) -> Result<()> {
    let parent = requested
        .parent()
        .context("authorized_keys has no parent")?;
    fs::create_dir_all(parent)?;
    set_mode(parent, 0o700)?;
    let path = if requested.exists() {
        requested.canonicalize().unwrap_or(requested)
    } else {
        requested
    };
    let parent = path.parent().context("authorized_keys has no parent")?;
    let lock_path = parent.join(".pi-authorized-keys.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&lock_path)?;
    lock.lock_exclusive()?;
    let original = fs::read_to_string(&path).unwrap_or_default();
    let remove_key = remove_key
        .map(PublicKey::to_openssh)
        .transpose()?
        .and_then(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.to_owned(), fields.next()?.to_owned()))
        });
    let mut next = original
        .lines()
        .filter(|line| {
            !remove_markers.iter().any(|marker| line.contains(*marker))
                && !remove_key
                    .as_ref()
                    .is_some_and(|key| line_contains_public_key(line, key))
        })
        .map(|line| format!("{line}\n"))
        .collect::<String>();
    if let Some(line) = append
        && !next.lines().any(|existing| existing == line.trim_end())
    {
        next.push_str(line);
    }
    if next != original {
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(next.as_bytes())?;
        temporary.as_file().sync_all()?;
        set_mode(temporary.path(), 0o600)?;
        temporary.persist(&path).map_err(|error| error.error)?;
    }
    fs2::FileExt::unlock(&lock)?;
    Ok(())
}

/// Finds the adjacent OpenSSH key type and base64 fields even when the entry
/// starts with quoted options whose forced command contains spaces.
fn line_contains_public_key(line: &str, key: &(String, String)) -> bool {
    let mut previous = None;
    for field in line.split_whitespace() {
        if previous == Some(key.0.as_str()) && field == key.1 {
            return true;
        }
        previous = Some(field);
    }
    false
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_private(path, &serde_json::to_vec(value)?, false)
}

fn write_new_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_private(path, &serde_json::to_vec(value)?, true)
}

fn write_private(path: &Path, bytes: &[u8], create_new: bool) -> Result<()> {
    let parent = path.parent().context("Pairing file has no parent")?;
    fs::create_dir_all(parent)?;
    set_mode(parent, 0o700)?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create(true)
        .truncate(!create_new)
        .create_new(create_new);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let file = OpenOptions::new().read(true).open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_REQUEST).read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn wait_for_done(id: &str, duration: Duration) -> Result<()> {
    let done = done_path(id)?;
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline && !done.exists() {
        thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

fn cleanup(state: &State) -> Result<()> {
    edit_authorized_keys(&[&state.bootstrap_marker], None, None)?;
    cleanup_files(&state.id);
    Ok(())
}

fn cleanup_files(id: &str) {
    for path in [
        state_path(id),
        request_path(id),
        response_path(id),
        done_path(id),
    ]
    .into_iter()
    .flatten()
    {
        fs::remove_file(path).ok();
    }
}

fn cleanup_expired() -> Result<()> {
    let directory = pairing_dir()?;
    let Ok(entries) = fs::read_dir(&directory) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().ends_with(".state.json") {
            continue;
        }
        let Ok(state) = read_json::<State>(&entry.path()) else {
            continue;
        };
        if state.expires_at < pairing::now() {
            cleanup(&state)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn forced_commands_quote_paths_and_gateway_is_allowlisted() {
        assert_eq!(
            forced_command("/tmp/Pi Desktop/helper", &["gateway", "phone"]).unwrap(),
            "\"exec '/tmp/Pi Desktop/helper' gateway phone\""
        );
        assert!(forced_command("bad\npath", &[]).is_err());
    }

    #[test]
    fn terminal_qr_forces_standard_black_on_white_colors() {
        let rendered = terminal_qr("pi://pair/v1#test").unwrap();
        assert!(rendered.starts_with("\x1b[30;47m"));
        assert!(rendered.ends_with("\x1b[0m"));
        assert!(rendered.contains('█'));
    }

    #[test]
    fn key_edits_preserve_unrelated_lines_and_replace_only_the_bootstrap() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        fs::write(
            &keys,
            "ssh-ed25519 existing user@host\nrestrict ssh-ed25519 old pi-pair-bootstrap:old\n",
        )
        .unwrap();
        edit_authorized_keys_at(
            keys.clone(),
            &["pi-pair-bootstrap:old"],
            None,
            Some("restrict ssh-ed25519 new pi-phone:new\n"),
        )
        .unwrap();
        let text = fs::read_to_string(&keys).unwrap();
        assert!(text.contains("ssh-ed25519 existing user@host"));
        assert!(text.contains("pi-phone:new"));
        assert!(!text.contains("pi-pair-bootstrap:old"));
    }

    #[test]
    fn qr_pairing_replaces_every_entry_with_the_same_phone_key() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        let public = private.public_key().clone();
        let openssh = public.to_openssh().unwrap();
        let encoded = openssh.split_whitespace().nth(1).unwrap();
        fs::write(
            &keys,
            format!(
                "ssh-ed25519 unrelated user@host\n{openssh} manual-copy\nrestrict,command=\"exec /old helper gateway old\" {openssh} pi-phone:old\n"
            ),
        )
        .unwrap();
        let (permanent, marker) = permanent_line("/new helper", public.clone()).unwrap();

        edit_authorized_keys_at(
            keys.clone(),
            &["pi-phone:old"],
            Some(&public),
            Some(&permanent),
        )
        .unwrap();

        let text = fs::read_to_string(&keys).unwrap();
        assert!(text.contains("ssh-ed25519 unrelated user@host"));
        assert!(text.contains("/new helper"));
        assert!(text.contains(&marker));
        assert!(!text.contains("manual-copy"));
        assert!(!text.contains("/old helper"));
        assert_eq!(text.matches(encoded).count(), 1);
    }

    #[test]
    fn requests_are_bound_to_the_offer_and_ed25519() {
        let state = State {
            id: "0123456789abcdef0123456789abcdef".into(),
            expires_at: pairing::now() + 30,
            bootstrap_marker: "marker".into(),
        };
        let key = PrivateKey::from(Ed25519Keypair::from_seed(&[3; 32]));
        let request = Request::new(
            state.id.clone(),
            key.public_key().to_openssh().unwrap(),
            "Phone".into(),
        );
        assert!(validate_request(&state, &request).is_ok());
        let (line, marker) = permanent_line("/tmp/pi helper", key.public_key().clone()).unwrap();
        assert!(line.contains("restrict,command="));
        assert!(line.contains(" gateway "));
        assert!(line.contains(&marker));
        assert!(marker.starts_with(PHONE_COMMENT));
        let mut wrong = request.clone();
        wrong.id = "fedcba9876543210fedcba9876543210".into();
        assert!(validate_request(&state, &wrong).is_err());
    }
}
