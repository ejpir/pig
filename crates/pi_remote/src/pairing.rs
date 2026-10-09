//! One-use QR enrollment through the computer's existing OpenSSH server.
//! No Pi TCP service listens: a restricted bootstrap key can only run `pair exchange`.

use anyhow::{Context as _, Result, bail, ensure};
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
    crate::activation::activate()?;
    let stable = match crate::activation::active_path() {
        Ok(stable) => stable,
        Err(_) => {
            // A helper launched from a build tree or release download cannot
            // become the canonical target of its new symlink in-place. Re-exec
            // the verified stable copy before writing any lasting key entry.
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                let stable = crate::activation::stable_path()?;
                let error = Command::new(&stable).arg("pair").args(arguments).exec();
                return Err(error).context("Could not continue pairing through the active helper");
            }
            #[cfg(not(unix))]
            unreachable!();
        }
    };
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
    let helper = helper_at(&stable, true)?;
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
    #[cfg(unix)]
    let executable = crate::activation::stable_path()?;
    #[cfg(not(unix))]
    let executable = env::current_exe()?
        .canonicalize()
        .context("The running helper cannot be discovered")?;
    helper_at(&executable, gateway)
}

fn helper_at(executable: &Path, gateway: bool) -> Result<Helper> {
    Ok(Helper {
        path: executable
            .to_str()
            .context("The helper path is not valid UTF-8")?
            .to_owned(),
        home: dirs::home_dir()
            .context("This account has no home folder")?
            .to_string_lossy()
            .trim_end_matches('/')
            .to_owned(),
        images: cfg!(feature = "bundled-durable"),
        gateway,
        release: Some(pi_core::ssh::VERSION.to_owned()),
        protocol: Some(pi_core::ssh::PROTOCOL_VERSION),
        platform: Some(crate::platform().to_owned()),
        capabilities: pairing::HelperCapabilities {
            pi: true,
            durable: cfg!(unix),
            durable_experimental: true,
            watchers: true,
            sessions: true,
            directories: true,
            commands: true,
            jj_history: true,
            delete_sessions: cfg!(unix),
            image_prompts: cfg!(feature = "bundled-durable"),
        },
    })
}

pub fn print_discovery(gateway: bool) -> Result<()> {
    println!("{}", serde_json::to_string(&helper(gateway)?)?);
    Ok(())
}

/// Returns the original app command accepted by a paired phone's forced command.
pub fn gateway_command() -> Result<Vec<String>> {
    let original = env::var("SSH_ORIGINAL_COMMAND").context("The SSH gateway needs a command")?;
    gateway_arguments(&original)
}

fn gateway_arguments(original: &str) -> Result<Vec<String>> {
    let mut command = shell_words::split(original).context("Invalid SSH command")?;
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
    let device = phone_marker(&key);
    key.set_comment(device.as_str());
    let forced = forced_command(path, &["gateway", &device])?;
    Ok((
        format!("restrict,command={forced} {}\n", key.to_openssh()?),
        device,
    ))
}

fn phone_marker(key: &PublicKey) -> String {
    let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
    let short: String = fingerprint
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(16)
        .collect();
    format!("{PHONE_COMMENT}{short}")
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
    let remove_key = remove_key
        .map(PublicKey::to_openssh)
        .transpose()?
        .and_then(|line| {
            let mut fields = line.split_whitespace();
            Some((fields.next()?.to_owned(), fields.next()?.to_owned()))
        });
    update_authorized_keys_at(requested, |original| {
        let mut next = Vec::with_capacity(original.len() + append.map_or(0, str::len));
        for raw in original.split_inclusive(|byte| *byte == b'\n') {
            let body = line_body(raw);
            let remove_marker =
                managed_entry(body).is_some_and(|entry| remove_markers.contains(&entry.marker));
            let remove_public_key = remove_key
                .as_ref()
                .is_some_and(|key| line_contains_public_key(body, key));
            if !remove_marker && !remove_public_key {
                next.extend_from_slice(raw);
            }
        }
        if let Some(line) = append {
            let wanted = line.trim_end_matches(['\r', '\n']).as_bytes();
            let exists = next
                .split_inclusive(|byte| *byte == b'\n')
                .any(|raw| line_body(raw) == wanted);
            if !exists {
                if !next.is_empty() && !next.ends_with(b"\n") {
                    next.push(b'\n');
                }
                next.extend_from_slice(line.as_bytes());
            }
        }
        Ok(next)
    })
}

/// Rewrites only entries emitted by `permanent_line`: an exact `pi-phone:`
/// comment whose exact forced command invokes `gateway` with the same marker.
pub(crate) fn migrate_phone_entries(stable: &Path) -> Result<usize> {
    migrate_phone_entries_at(authorized_keys_path()?, stable)
}

fn migrate_phone_entries_at(requested: PathBuf, stable: &Path) -> Result<usize> {
    match fs::symlink_metadata(&requested) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    }
    let stable = stable
        .to_str()
        .context("The stable helper path is not valid UTF-8")?;
    let mut migrated = 0;
    update_authorized_keys_at(requested, |original| {
        let mut next = Vec::with_capacity(original.len());
        for raw in original.split_inclusive(|byte| *byte == b'\n') {
            let body = line_body(raw);
            let Some(entry) = managed_entry(body).filter(|entry| entry.kind == ManagedKind::Phone)
            else {
                next.extend_from_slice(raw);
                continue;
            };
            let replacement = format!(
                "command={}",
                forced_command(stable, &["gateway", entry.marker])?
            );
            let option = &entry.command_option;
            if entry.text[option.clone()] == replacement {
                next.extend_from_slice(raw);
                continue;
            }
            next.extend_from_slice(&entry.text.as_bytes()[..option.start]);
            next.extend_from_slice(replacement.as_bytes());
            next.extend_from_slice(&entry.text.as_bytes()[option.end..]);
            next.extend_from_slice(&raw[body.len()..]);
            migrated += 1;
        }
        Ok(next)
    })?;
    Ok(migrated)
}

fn update_authorized_keys_at(
    requested: PathBuf,
    update: impl FnOnce(&[u8]) -> Result<Vec<u8>>,
) -> Result<()> {
    let requested_parent = requested
        .parent()
        .context("authorized_keys has no parent")?;
    fs::create_dir_all(requested_parent)?;
    set_mode(requested_parent, 0o700)?;
    let path = match requested.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => requested,
        Err(error) => return Err(error.into()),
    };
    let parent = path.parent().context("authorized_keys has no parent")?;
    let lock_path = parent.join(".pi-authorized-keys.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let lock = options.open(&lock_path)?;
    ensure!(
        lock.metadata()?.is_file(),
        "The authorized_keys lock is not a regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        lock.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    set_mode(&lock_path, 0o600)?;
    lock.lock_exclusive()?;
    let original = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let next = update(&original)?;
    if next != original {
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        temporary.write_all(&next)?;
        temporary.as_file().sync_all()?;
        set_mode(temporary.path(), 0o600)?;
        temporary.persist(&path).map_err(|error| error.error)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
    }
    fs2::FileExt::unlock(&lock)?;
    Ok(())
}

fn line_body(raw: &[u8]) -> &[u8] {
    let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
    raw.strip_suffix(b"\r").unwrap_or(raw)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedKind {
    Bootstrap,
    Phone,
}

struct ManagedEntry<'a> {
    text: &'a str,
    command_option: std::ops::Range<usize>,
    marker: &'a str,
    kind: ManagedKind,
}

fn managed_entry(line: &[u8]) -> Option<ManagedEntry<'_>> {
    let text = std::str::from_utf8(line).ok()?;
    let fields = authorized_fields(text)?;
    let (key_index, key) = public_key(text, &fields)?;
    if key_index != 1 || fields.len() != 4 || key.algorithm() != Algorithm::Ed25519 {
        return None;
    }
    let options = &text[fields[0].clone()];
    let option_ranges = authorized_options(options)?;
    if !option_ranges
        .iter()
        .any(|range| &options[range.clone()] == "restrict")
    {
        return None;
    }
    let commands: Vec<_> = option_ranges
        .iter()
        .filter(|range| options[(*range).clone()].starts_with("command="))
        .cloned()
        .collect();
    let [command_option] = commands.as_slice() else {
        return None;
    };
    let command = forced_option_command(&options[command_option.clone()])?;
    let command = shell_words::split(&command).ok()?;
    let marker = &text[fields[3].clone()];
    let program = command.get(1).map(Path::new)?;
    if !program.is_absolute() || !managed_helper_name(program.file_name()?.to_str()?) {
        return None;
    }
    let kind = match command.as_slice() {
        [exec, _program, gateway, argument]
            if exec == "exec"
                && gateway == "gateway"
                && argument == marker
                && marker == phone_marker(&key) =>
        {
            ManagedKind::Phone
        }
        [exec, _program, pair, exchange, id]
            if exec == "exec"
                && pair == "pair"
                && exchange == "exchange"
                && marker == format!("{BOOTSTRAP_COMMENT}{id}")
                && valid_id(id).is_ok() =>
        {
            ManagedKind::Bootstrap
        }
        _ => return None,
    };
    Some(ManagedEntry {
        text,
        command_option: fields[0].start + command_option.start
            ..fields[0].start + command_option.end,
        marker,
        kind,
    })
}

fn managed_helper_name(name: &str) -> bool {
    matches!(
        name,
        "pi-desktop-remote"
            | "pi-desktop-remote-linux-amd64"
            | "pi-desktop-remote-linux-arm64"
            | "pi-desktop-remote-macos-arm64"
    )
}

fn authorized_options(options: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let bytes = options.as_bytes();
    let mut ranges = Vec::new();
    let mut start = 0;
    let mut cursor = 0;
    let mut quoted = false;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'\\' if quoted => {
                cursor += 1;
                if cursor == bytes.len() {
                    return None;
                }
            }
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                if start == cursor {
                    return None;
                }
                ranges.push(start..cursor);
                start = cursor + 1;
            }
            _ => {}
        }
        cursor += 1;
    }
    if quoted || start == bytes.len() {
        return None;
    }
    ranges.push(start..bytes.len());
    Some(ranges)
}

fn forced_option_command(option: &str) -> Option<String> {
    let quoted = option.strip_prefix("command=\"")?.strip_suffix('"')?;
    let mut command = String::with_capacity(quoted.len());
    let mut bytes = quoted.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'\\' {
            let escaped = bytes.next()?;
            if escaped != b'\\' && escaped != b'"' {
                return None;
            }
            command.push(escaped as char);
        } else if byte == b'"' {
            return None;
        } else {
            command.push(byte as char);
        }
    }
    Some(command)
}

fn authorized_fields(line: &str) -> Option<Vec<std::ops::Range<usize>>> {
    let bytes = line.as_bytes();
    let mut fields = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor == bytes.len() {
            break;
        }
        let start = cursor;
        let mut quoted = false;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\\' if quoted => {
                    cursor += 1;
                    if cursor == bytes.len() {
                        return None;
                    }
                    cursor += 1;
                }
                b'"' => {
                    quoted = !quoted;
                    cursor += 1;
                }
                byte if !quoted && byte.is_ascii_whitespace() => break,
                _ => cursor += 1,
            }
        }
        if quoted {
            return None;
        }
        fields.push(start..cursor);
    }
    Some(fields)
}

fn public_key(line: &str, fields: &[std::ops::Range<usize>]) -> Option<(usize, PublicKey)> {
    fields.windows(2).enumerate().find_map(|(index, pair)| {
        let candidate = format!("{} {}", &line[pair[0].clone()], &line[pair[1].clone()]);
        PublicKey::from_openssh(&candidate)
            .ok()
            .map(|key| (index, key))
    })
}

/// Finds the adjacent OpenSSH key type and base64 fields even when the entry
/// starts with quoted options whose forced command contains spaces.
fn line_contains_public_key(line: &[u8], key: &(String, String)) -> bool {
    let Ok(text) = std::str::from_utf8(line) else {
        return false;
    };
    let Some(fields) = authorized_fields(text) else {
        return false;
    };
    fields
        .windows(2)
        .any(|pair| text[pair[0].clone()] == key.0 && text[pair[1].clone()] == key.1)
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
    fn key_edits_preserve_bytes_and_remove_only_an_exact_managed_bootstrap() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let id = "0123456789abcdef0123456789abcdef";
        let marker = format!("{BOOTSTRAP_COMMENT}{id}");
        let mut bootstrap = PrivateKey::from(Ed25519Keypair::from_seed(&[5; 32]));
        bootstrap.set_comment(marker.clone());
        let forced =
            forced_command("/old/helper/pi-desktop-remote", &["pair", "exchange", id]).unwrap();
        let managed = format!(
            "restrict,command={forced} {}\r\n",
            bootstrap.public_key().to_openssh().unwrap()
        );
        let unrelated = format!("ssh-ed25519 not-a-key note-{marker}\n");
        let mut original = unrelated.as_bytes().to_vec();
        original.extend_from_slice(b"# non-utf8: \xff\n");
        original.extend_from_slice(managed.as_bytes());
        fs::write(&keys, &original).unwrap();

        let append_key = PrivateKey::from(Ed25519Keypair::from_seed(&[6; 32]));
        let (append, _) = permanent_line("/new helper", append_key.public_key().clone()).unwrap();
        edit_authorized_keys_at(keys.clone(), &[&marker], None, Some(&append)).unwrap();

        let bytes = fs::read(&keys).unwrap();
        assert!(bytes.starts_with(unrelated.as_bytes()));
        assert!(
            bytes
                .windows(b"non-utf8: \xff".len())
                .any(|window| window == b"non-utf8: \xff")
        );
        assert!(
            !bytes
                .windows(managed.len())
                .any(|window| window == managed.as_bytes())
        );
        assert!(String::from_utf8_lossy(&bytes).contains("/new helper"));
    }

    #[cfg(unix)]
    #[test]
    fn authorized_keys_lock_never_follows_a_symlink() {
        use std::os::unix::fs::symlink;

        let directory = isolated();
        let keys = directory.path().join(".ssh/authorized_keys");
        fs::create_dir_all(keys.parent().unwrap()).unwrap();
        fs::write(&keys, b"keep\n").unwrap();
        let victim = directory.path().join("victim");
        fs::write(&victim, b"victim\n").unwrap();
        symlink(
            &victim,
            keys.parent().unwrap().join(".pi-authorized-keys.lock"),
        )
        .unwrap();

        assert!(edit_authorized_keys_at(keys.clone(), &[], None, Some("append\n")).is_err());
        assert_eq!(fs::read(&keys).unwrap(), b"keep\n");
        assert_eq!(fs::read(&victim).unwrap(), b"victim\n");
    }

    #[test]
    fn qr_pairing_replaces_every_entry_with_the_same_phone_key() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        let public = private.public_key().clone();
        let openssh = public.to_openssh().unwrap();
        let encoded = openssh.split_whitespace().nth(1).unwrap();
        let (old, old_marker) = permanent_line("/old helper", public.clone()).unwrap();
        fs::write(
            &keys,
            format!("ssh-ed25519 unrelated user@host\n{openssh} manual-copy\n{old}"),
        )
        .unwrap();
        let (permanent, marker) = permanent_line("/new helper", public.clone()).unwrap();

        edit_authorized_keys_at(
            keys.clone(),
            &[&old_marker],
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
    fn managed_helper_names_are_exact_release_artifacts() {
        for name in [
            "pi-desktop-remote",
            "pi-desktop-remote-linux-amd64",
            "pi-desktop-remote-linux-arm64",
            "pi-desktop-remote-macos-arm64",
        ] {
            assert!(managed_helper_name(name));
        }
        assert!(!managed_helper_name("pi-desktop-remote-malware"));
        assert!(!managed_helper_name("not-pi-desktop-remote"));
    }

    #[test]
    fn migration_changes_only_recognized_phone_gateways_and_is_idempotent() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[9; 32]));
        let (old, marker) =
            permanent_line("/old/hash/pi-desktop-remote", private.public_key().clone()).unwrap();
        let old = old
            .replacen(
                "restrict,command=",
                "from=\"10.0.0.0/8\",restrict,command=",
                1,
            )
            .replacen(" ssh-ed25519 ", ",no-port-forwarding ssh-ed25519 ", 1);
        let wrong_program = old.replace(
            "/old/hash/pi-desktop-remote",
            "/old/hash/not-pi-desktop-remote",
        );
        let prefixed_program = old.replace(
            "/old/hash/pi-desktop-remote",
            "/old/hash/pi-desktop-remote-malware",
        );
        let mismatched = old.replace(
            &format!("gateway {marker}"),
            "gateway pi-phone:someone-else",
        );
        let forged_marker = old.replace(&marker, "pi-phone:0000000000000000");
        let unrestricted = old.replacen("restrict,", "", 1);
        let mut original = b"# keep this exact\r\n\xffbinary\n".to_vec();
        original.extend_from_slice(wrong_program.as_bytes());
        original.extend_from_slice(prefixed_program.as_bytes());
        original.extend_from_slice(mismatched.as_bytes());
        original.extend_from_slice(forged_marker.as_bytes());
        original.extend_from_slice(unrestricted.as_bytes());
        original.extend_from_slice(old.as_bytes());
        fs::write(&keys, &original).unwrap();
        let stable = Path::new("/home/me/.pi/desktop/bin/pi-desktop-remote");

        assert_eq!(migrate_phone_entries_at(keys.clone(), stable).unwrap(), 1);
        let once = fs::read(&keys).unwrap();
        assert!(once.starts_with(b"# keep this exact\r\n\xffbinary\n"));
        for unchanged in [
            &wrong_program,
            &prefixed_program,
            &mismatched,
            &forged_marker,
            &unrestricted,
        ] {
            assert!(
                once.windows(unchanged.len())
                    .any(|window| window == unchanged.as_bytes())
            );
        }
        let migrated = String::from_utf8_lossy(&once);
        assert!(migrated.contains("from=\"10.0.0.0/8\",restrict,command="));
        assert!(migrated.contains(",no-port-forwarding ssh-ed25519 "));
        assert!(migrated.contains(&format!("exec {} gateway {marker}", stable.display())));
        assert_eq!(migrate_phone_entries_at(keys.clone(), stable).unwrap(), 0);
        assert_eq!(fs::read(&keys).unwrap(), once);
    }

    #[test]
    fn stable_pairing_commands_use_the_activated_absolute_path() {
        let stable = Path::new("/home/me/.pi/desktop/bin/pi-desktop-remote");
        let helper = helper_at(stable, true).unwrap();
        assert_eq!(helper.path, stable.to_str().unwrap());
        let key = PrivateKey::from(Ed25519Keypair::from_seed(&[11; 32]));
        let (line, marker) = permanent_line(&helper.path, key.public_key().clone()).unwrap();
        assert!(line.contains(&format!("exec {} gateway {marker}", stable.display())));
        let bootstrap = forced_command(
            &helper.path,
            &["pair", "exchange", "0123456789abcdef0123456789abcdef"],
        )
        .unwrap();
        assert!(bootstrap.contains(&format!("exec {} pair exchange", stable.display())));
    }

    #[test]
    fn the_forced_gateway_never_allows_activation() {
        assert!(gateway_arguments("pi-desktop-remote sessions").is_ok());
        assert!(gateway_arguments("pi-desktop-remote activate").is_err());
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
