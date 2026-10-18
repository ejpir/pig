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
const AUTHORIZED_KEYS_RECOVERY_PREFIX: &str = ".pi-authorized-keys-recovery-";
const MAX_AUTHORIZED_KEYS_RECOVERIES: usize = 64;

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

/// Rolls a newly installed phone authorization back unless its approval
/// response was durably published and its forced-command commit marker exists.
struct PhoneAuthorization {
    path: PathBuf,
    authorization_id: String,
    installed: Vec<u8>,
    armed: bool,
}

impl PhoneAuthorization {
    fn publish(&mut self) -> Result<()> {
        write_private(
            &authorization_path(&self.authorization_id)?,
            b"authorized\n",
            true,
        )?;
        self.armed = false;
        Ok(())
    }

    fn rollback(&mut self) -> Result<()> {
        if self.armed {
            rollback_phone_authorization(&self.path, &self.installed)?;
            self.armed = false;
        }
        Ok(())
    }
}

impl Drop for PhoneAuthorization {
    fn drop(&mut self) {
        self.rollback().ok();
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
    if !cfg!(any(target_os = "linux", target_os = "macos")) {
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
    // Arm cleanup before publication. Reserve one ordinary recovery slot for
    // the phone-key install; compensating removals bypass the admission cap.
    let _bootstrap = BootstrapGuard(state.clone());
    edit_authorized_keys_reserved(&[&marker], None, Some(&line), 1)?;
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
        println!("This grants Pi access plus an interactive shell as this account.");
        print!("Type y or yes to allow, anything else to deny: ");
        io::stdout().flush()?;
        read_confirmation(&mut io::stdin().lock())?
    };

    if approved && pairing::now() > state.expires_at {
        cleanup(&state)?;
        bail!("Pairing expired before approval; no phone key was authorized");
    }

    let mut authorization = None;
    let response = if approved {
        let authorization_id = random_id();
        let (permanent, phone_marker) =
            staged_permanent_line(&helper.path, phone_key.clone(), &authorization_id)?;
        // Re-pairing the same phone upgrades its forced command to this helper
        // instead of leaving an older matching key earlier in the file. Match
        // the key material too: a key copied during manual setup has a different
        // comment and no Pi marker, but OpenSSH would otherwise accept that
        // unrestricted entry before reaching the gateway entry below it.
        authorization = Some(install_phone_authorization(
            &marker,
            &phone_marker,
            phone_key,
            &permanent,
            authorization_id,
        )?);
        Response {
            version: VERSION,
            approved: true,
            error: None,
            helper: Some(helper),
        }
    } else {
        edit_authorized_keys_cleanup(&[&marker], None)?;
        Response {
            version: VERSION,
            approved: false,
            error: Some("Pairing was denied on the computer".into()),
            helper: None,
        }
    };
    if let Err(error) = write_json(&response_path(&id)?, &response) {
        if let Some(authorization) = &mut authorization {
            authorization
                .rollback()
                .context("Pairing response failed and the phone key could not be rolled back")?;
        }
        return Err(error.context("Could not publish the pairing response"));
    }
    if let Some(authorization) = &mut authorization {
        authorization
            .publish()
            .context("Could not activate the published phone authorization")?;
    }
    wait_for_done(&id, Duration::from_secs(10))?;
    cleanup_files(&id);
    if approved {
        println!("Paired. The phone can access Pi and open a shell as this account.");
        Ok(())
    } else {
        bail!("Pairing denied; no phone key was authorized")
    }
}

fn read_confirmation(input: &mut impl io::BufRead) -> Result<bool> {
    let mut answer = String::new();
    let read = input.read_line(&mut answer)?;
    Ok(read != 0 && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
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
            edit_authorized_keys_cleanup(&[&marker], None).ok();
            cleanup_files(id);
            return Err(error).context("This pairing code is no longer active");
        }
    };
    if state.id != id || pairing::now() > state.expires_at {
        edit_authorized_keys_cleanup(&[&marker], None).ok();
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
            schedules: cfg!(any(target_os = "linux", target_os = "macos")),
            durable_experimental: true,
            watchers: true,
            sessions: true,
            directories: true,
            file_editing: cfg!(any(target_os = "linux", target_os = "macos")),
            commands: true,
            jj_history: true,
            delete_sessions: cfg!(unix),
            image_prompts: cfg!(feature = "bundled-durable"),
            terminal: cfg!(target_os = "linux"),
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
        (one == "connect" || one == "files" || one == "schedules") && two == "--stdio"
    ) || command.as_slice() == ["terminal", "--stdio"]
        || matches!(command.as_slice(), [one, two, _] if one == "directories" && two == "--path")
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

#[cfg(test)]
fn permanent_line(path: &str, mut key: PublicKey) -> Result<(String, String)> {
    let device = phone_marker(&key);
    key.set_comment(device.as_str());
    let forced = forced_command(path, &["gateway", "2", &device])?;
    Ok((
        format!("restrict,command={forced} {}\n", key.to_openssh()?),
        device,
    ))
}

fn staged_permanent_line(
    path: &str,
    mut key: PublicKey,
    authorization_id: &str,
) -> Result<(String, String)> {
    valid_id(authorization_id)?;
    let device = phone_marker(&key);
    key.set_comment(device.as_str());
    let forced = forced_command(path, &["gateway", "3", &device, authorization_id])?;
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

fn authorization_path(id: &str) -> Result<PathBuf> {
    valid_id(id)?;
    Ok(dirs::home_dir()
        .context("This account has no home folder")?
        .join(".pi/desktop/authorizations")
        .join(format!("{id}.authorized")))
}

pub fn require_gateway_authorization(id: &str) -> Result<()> {
    require_gateway_authorization_at(&authorization_path(id)?)
}

fn require_gateway_authorization_at(path: &Path) -> Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options
        .open(path)
        .context("This phone authorization is not committed")?;
    ensure!(
        file.metadata()?.is_file(),
        "The phone authorization marker is not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(32).read_to_end(&mut bytes)?;
    ensure!(
        bytes == b"authorized\n",
        "The phone authorization marker is invalid"
    );
    Ok(())
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

fn edit_authorized_keys_reserved(
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
    append: Option<&str>,
    reserve_recoveries: usize,
) -> Result<()> {
    edit_authorized_keys_at_reserved(
        authorized_keys_path()?,
        remove_markers,
        remove_key,
        append,
        reserve_recoveries,
    )
}

fn edit_authorized_keys_cleanup(
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
) -> Result<()> {
    edit_authorized_keys_at_with_quota(
        authorized_keys_path()?,
        remove_markers,
        remove_key,
        None,
        None,
    )
}

fn install_phone_authorization(
    bootstrap_marker: &str,
    phone_marker: &str,
    phone_key: PublicKey,
    permanent: &str,
    authorization_id: String,
) -> Result<PhoneAuthorization> {
    let path = authorized_keys_path()?;
    let encoded_key = public_key_fields(&phone_key)?;
    let mut attempted = false;
    let update = update_authorized_keys_at_reserved(path.clone(), 0, |original| {
        attempted = true;
        let mut next = Vec::with_capacity(original.len() + permanent.len());
        for raw in original.split_inclusive(|byte| *byte == b'\n') {
            let body = line_body(raw);
            let remove_marker = managed_entry(body).is_some_and(|entry| {
                entry.marker == bootstrap_marker || entry.marker == phone_marker
            });
            if !remove_marker && !line_contains_public_key(body, &encoded_key) {
                next.extend_from_slice(raw);
            }
        }
        append_authorized_line(&mut next, permanent);
        Ok(next)
    });
    let mut authorization = PhoneAuthorization {
        path,
        authorization_id,
        installed: permanent.as_bytes().to_vec(),
        armed: attempted,
    };
    if let Err(error) = update {
        if let Err(rollback) = authorization.rollback() {
            return Err(error.context(format!(
                "Phone authorization publication failed and rollback also failed: {rollback:#}"
            )));
        }
        return Err(error);
    }
    authorization.armed = true;
    Ok(authorization)
}

fn rollback_phone_authorization(path: &Path, installed: &[u8]) -> Result<()> {
    let installed = line_body(installed);
    update_authorized_keys_at_with_quota(path.to_owned(), None, |current| {
        if !current
            .split_inclusive(|byte| *byte == b'\n')
            .any(|raw| line_body(raw) == installed)
        {
            return Ok(current.to_vec());
        }
        // Rollback is removal-only. Restoring an older same-key authorization
        // would transiently republish it if an external revocation raced the
        // compare/exchange. The displaced generation remains in recovery.
        let mut next = Vec::with_capacity(current.len());
        for raw in current.split_inclusive(|byte| *byte == b'\n') {
            if line_body(raw) != installed {
                next.extend_from_slice(raw);
            }
        }
        Ok(next)
    })
}

fn public_key_fields(key: &PublicKey) -> Result<(String, String)> {
    let line = key.to_openssh()?;
    let mut fields = line.split_whitespace();
    Ok((
        fields
            .next()
            .context("Public key has no algorithm")?
            .to_owned(),
        fields.next().context("Public key has no data")?.to_owned(),
    ))
}

fn append_authorized_line(next: &mut Vec<u8>, line: &str) {
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

#[cfg(test)]
fn edit_authorized_keys_at(
    requested: PathBuf,
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
    append: Option<&str>,
) -> Result<()> {
    edit_authorized_keys_at_reserved(requested, remove_markers, remove_key, append, 0)
}

fn edit_authorized_keys_at_reserved(
    requested: PathBuf,
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
    append: Option<&str>,
    reserve_recoveries: usize,
) -> Result<()> {
    edit_authorized_keys_at_with_quota(
        requested,
        remove_markers,
        remove_key,
        append,
        Some(reserve_recoveries),
    )
}

fn edit_authorized_keys_at_with_quota(
    requested: PathBuf,
    remove_markers: &[&str],
    remove_key: Option<&PublicKey>,
    append: Option<&str>,
    recovery_quota: Option<usize>,
) -> Result<()> {
    let remove_key = remove_key.map(public_key_fields).transpose()?;
    update_authorized_keys_at_with_quota(requested, recovery_quota, |original| {
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
            append_authorized_line(&mut next, line);
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
        migrated = 0;
        let mut next = Vec::with_capacity(original.len());
        for raw in original.split_inclusive(|byte| *byte == b'\n') {
            let body = line_body(raw);
            let Some(entry) = managed_entry(body).filter(|entry| entry.kind == ManagedKind::Phone)
            else {
                next.extend_from_slice(raw);
                continue;
            };
            let forced = match entry.authorization_id.as_deref() {
                Some(authorization_id) => {
                    forced_command(stable, &["gateway", "3", entry.marker, authorization_id])?
                }
                None => forced_command(stable, &["gateway", "2", entry.marker])?,
            };
            let replacement = format!("command={forced}");
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
    update: impl FnMut(&[u8]) -> Result<Vec<u8>>,
) -> Result<()> {
    update_authorized_keys_at_reserved(requested, 0, update)
}

fn update_authorized_keys_at_reserved(
    requested: PathBuf,
    reserve_recoveries: usize,
    update: impl FnMut(&[u8]) -> Result<Vec<u8>>,
) -> Result<()> {
    update_authorized_keys_at_with_quota(requested, Some(reserve_recoveries), update)
}

fn update_authorized_keys_at_with_quota(
    requested: PathBuf,
    recovery_quota: Option<usize>,
    mut update: impl FnMut(&[u8]) -> Result<Vec<u8>>,
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
    for _ in 0..16 {
        let (original, existed) = read_authorized_keys(&path)?;
        let next = update(&original)?;
        if next == original {
            fs2::FileExt::unlock(&lock)?;
            return Ok(());
        }
        let recoveries = fs::read_dir(parent)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(AUTHORIZED_KEYS_RECOVERY_PREFIX)
            })
            .count();
        if let Some(reserve_recoveries) = recovery_quota {
            ensure!(
                recoveries
                    .saturating_add(1)
                    .saturating_add(reserve_recoveries)
                    <= MAX_AUTHORIZED_KEYS_RECOVERIES,
                "Too many authorized_keys recovery files in {}; inspect and remove obsolete {}* files",
                parent.display(),
                AUTHORIZED_KEYS_RECOVERY_PREFIX
            );
        }
        let mut temporary = tempfile::Builder::new()
            .prefix(AUTHORIZED_KEYS_RECOVERY_PREFIX)
            .tempfile_in(parent)?;
        temporary.write_all(&next)?;
        temporary.as_file().sync_all()?;
        set_mode(temporary.path(), 0o600)?;
        let exchanged =
            match compare_exchange_authorized_keys(temporary.path(), &path, &original, existed) {
                Ok(exchanged) => exchanged,
                Err(error) => {
                    // Once an exchange has started, this pathname may contain a
                    // displaced external edit rather than Pi's proposal. Never let
                    // NamedTempFile unlink it on an error: keep it as an explicit
                    // recovery record beside authorized_keys and report its path.
                    let (recovery, keep_error) = preserve_authorized_keys_recovery(temporary);
                    let sync_error = sync_directory(parent).err();
                    fs2::FileExt::unlock(&lock).ok();
                    let mut detail = format!(
                        "authorized_keys recovery was preserved at {}",
                        recovery.display()
                    );
                    if let Some(keep_error) = keep_error {
                        detail.push_str(&format!(" (could not mark it persistent: {keep_error})"));
                    }
                    if let Some(sync_error) = sync_error {
                        detail.push_str(&format!(" (directory sync failed: {sync_error})"));
                    }
                    return Err(error.context(detail));
                }
            };
        if exchanged {
            // The displaced inode can still have external writers holding old
            // descriptors. Keep it reachable so even writes that arrive after
            // our final comparison cannot disappear when this scope exits.
            let recovery = existed.then(|| preserve_authorized_keys_recovery(temporary));
            if let Err(error) = sync_directory(parent) {
                eprintln!(
                    "Warning: authorized_keys was published but its directory could not be synced: {error}"
                );
            }
            if let Err(error) = fs2::FileExt::unlock(&lock) {
                eprintln!(
                    "Warning: authorized_keys was published but its advisory lock could not be explicitly released: {error}"
                );
            }
            if let Some((recovery, Some(error))) = recovery {
                eprintln!(
                    "Warning: authorized_keys recovery remains at {} but could not be marked persistent: {error}",
                    recovery.display()
                );
            }
            return Ok(());
        }
        if existed {
            // A failed comparison also made Pi's proposal briefly live. Keep
            // that provisional inode reachable because an external writer may
            // still hold it and write after restoration validation completes.
            let (recovery, keep_error) = preserve_authorized_keys_recovery(temporary);
            if let Some(error) = keep_error {
                eprintln!(
                    "Warning: provisional authorized_keys recovery remains at {} but could not be marked persistent: {error}",
                    recovery.display()
                );
            }
            if let Err(error) = sync_directory(parent) {
                eprintln!(
                    "Warning: provisional authorized_keys recovery could not be synced: {error}"
                );
            }
        }
    }
    fs2::FileExt::unlock(&lock)?;
    bail!("authorized_keys kept changing while Pi tried to update it")
}

fn preserve_authorized_keys_recovery(
    temporary: tempfile::NamedTempFile,
) -> (PathBuf, Option<String>) {
    let recovery = temporary.path().to_path_buf();
    match temporary.keep() {
        Ok((_file, path)) => (path, None),
        Err(keep_error) => {
            // `keep` normally only disables automatic unlinking. If even that
            // bookkeeping fails, leak the owner intentionally: dropping it
            // would destroy the only pathname for a displaced generation.
            let message = keep_error.error.to_string();
            std::mem::forget(keep_error.file);
            (recovery, Some(message))
        }
    }
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn read_authorized_keys(path: &Path) -> Result<(Vec<u8>, bool)> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), false)),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.is_file(),
        "authorized_keys is not a regular file"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok((bytes, true))
}

/// Installs `temporary` only if `path` still contains the bytes that were used
/// to derive it. Atomic exchange makes the displaced live file inspectable; a
/// concurrent external edit is exchanged straight back and retried instead of
/// being overwritten.
fn compare_exchange_authorized_keys(
    temporary: &Path,
    path: &Path,
    expected: &[u8],
    existed: bool,
) -> Result<bool> {
    if existed {
        let installed = inode_identity(temporary)?;
        match rename_exchange(temporary, path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        }
        let displaced = read_authorized_keys(temporary);
        match displaced {
            Ok((bytes, true)) if bytes == expected => return Ok(true),
            Ok(_) => {}
            Err(error) => {
                restore_displaced_authorized_keys(temporary, path, installed)
                    .context("Could not restore authorized_keys after comparison failed")?;
                return Err(error.context("Could not inspect displaced authorized_keys"));
            }
        }
        restore_displaced_authorized_keys(temporary, path, installed)
            .context("Could not restore a concurrently edited authorized_keys")?;
        Ok(false)
    } else {
        match rename_no_replace(temporary, path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct InodeIdentity {
    device: u64,
    inode: u64,
    length: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    mode: u32,
    user: u32,
    group: u32,
    digest: [u8; 32],
    // Pin ordinary files so a removed inode cannot be reused while it is the
    // generation against which an exchange is being checked.
    _pin: fs::File,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl InodeIdentity {
    fn same_generation(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.length == other.length
            && self.modified_seconds == other.modified_seconds
            && self.modified_nanoseconds == other.modified_nanoseconds
            && self.mode == other.mode
            && self.user == other.user
            && self.group == other.group
            && self.digest == other.digest
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn inode_identity(path: &Path) -> Result<InodeIdentity> {
    use sha2::{Digest as _, Sha256};
    use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

    for _ in 0..16 {
        let before = fs::symlink_metadata(path)?;
        ensure!(before.is_file(), "authorized_keys is not a regular file");
        let mut pin_options = OpenOptions::new();
        pin_options.read(true).custom_flags(libc::O_NOFOLLOW);
        let pin = pin_options.open(path)?;
        let pinned = pin.metadata()?;
        if before.dev() != pinned.dev() || before.ino() != pinned.ino() {
            continue;
        }

        let mut reader = &pin;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 8192];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let digest = hasher.finalize().into();

        let after = pin.metadata()?;
        if pinned.dev() == after.dev()
            && pinned.ino() == after.ino()
            && pinned.len() == after.len()
            && pinned.ctime() == after.ctime()
            && pinned.ctime_nsec() == after.ctime_nsec()
            && pinned.mtime() == after.mtime()
            && pinned.mtime_nsec() == after.mtime_nsec()
        {
            return Ok(InodeIdentity {
                device: after.dev(),
                inode: after.ino(),
                length: after.len(),
                modified_seconds: after.mtime(),
                modified_nanoseconds: after.mtime_nsec(),
                mode: after.mode(),
                user: after.uid(),
                group: after.gid(),
                digest,
                _pin: pin,
            });
        }
    }
    bail!("authorized_keys generation kept changing while Pi inspected it")
}

/// Restores the newest external generation displaced by a failed compare and
/// exchange. Each exchange is checked by inode identity. If another editor
/// replaces `path` during restoration, its newly displaced generation is
/// rotated back next instead of being deleted with the temporary file.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn restore_displaced_authorized_keys(
    temporary: &Path,
    path: &Path,
    mut expected_live: InodeIdentity,
) -> Result<()> {
    let mut candidate = inode_identity(temporary)?;
    for _ in 0..64 {
        match rename_exchange(temporary, path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // A concurrent deletion of the live path is a newer edit and
                // wins. Distinguish it from interference with our temporary.
                return match fs::symlink_metadata(path) {
                    Err(missing) if missing.kind() == io::ErrorKind::NotFound => Ok(()),
                    _ => Err(error.into()),
                };
            }
            Err(error) => return Err(error.into()),
        }
        let displaced = inode_identity(temporary)?;
        if displaced.same_generation(&expected_live) {
            return Ok(());
        }
        expected_live = candidate;
        candidate = displaced;
    }
    bail!("authorized_keys kept changing while Pi restored an external edit")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
struct InodeIdentity;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn inode_identity(_: &Path) -> Result<InodeIdentity> {
    bail!("atomic authorized_keys exchange is unsupported")
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn restore_displaced_authorized_keys(_: &Path, _: &Path, _: InodeIdentity) -> Result<()> {
    bail!("atomic authorized_keys exchange is unsupported")
}

#[cfg(target_os = "linux")]
fn rename_exchange(left: &Path, right: &Path) -> io::Result<()> {
    renameat2(left, right, libc::RENAME_EXCHANGE)
}

#[cfg(target_os = "linux")]
fn rename_no_replace(left: &Path, right: &Path) -> io::Result<()> {
    renameat2(left, right, libc::RENAME_NOREPLACE)
}

#[cfg(target_os = "linux")]
fn renameat2(left: &Path, right: &Path, flags: libc::c_uint) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt as _};
    let left = CString::new(left.as_os_str().as_bytes())?;
    let right = CString::new(right.as_os_str().as_bytes())?;
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            flags,
        )
    } == -1
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn rename_exchange(left: &Path, right: &Path) -> io::Result<()> {
    renameatx_np(left, right, libc::RENAME_SWAP)
}

#[cfg(target_os = "macos")]
fn rename_no_replace(left: &Path, right: &Path) -> io::Result<()> {
    renameatx_np(left, right, libc::RENAME_EXCL)
}

#[cfg(target_os = "macos")]
fn renameatx_np(left: &Path, right: &Path, flags: libc::c_uint) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt as _};
    let left = CString::new(left.as_os_str().as_bytes())?;
    let right = CString::new(right.as_os_str().as_bytes())?;
    if unsafe {
        libc::renameatx_np(
            libc::AT_FDCWD,
            left.as_ptr(),
            libc::AT_FDCWD,
            right.as_ptr(),
            flags,
        )
    } == -1
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_exchange(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic authorized_keys exchange is unsupported",
    ))
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn rename_no_replace(_: &Path, _: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic authorized_keys replacement is unsupported",
    ))
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
    authorization_id: Option<String>,
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
    let (kind, authorization_id) = match command.as_slice() {
        [exec, _program, gateway, argument]
            if exec == "exec"
                && gateway == "gateway"
                && argument == marker
                && marker == phone_marker(&key) =>
        {
            (ManagedKind::Phone, None)
        }
        [exec, _program, gateway, version, argument]
            if exec == "exec"
                && gateway == "gateway"
                && version == "2"
                && argument == marker
                && marker == phone_marker(&key) =>
        {
            (ManagedKind::Phone, None)
        }
        [exec, _program, gateway, version, argument, authorization_id]
            if exec == "exec"
                && gateway == "gateway"
                && version == "3"
                && argument == marker
                && marker == phone_marker(&key)
                && valid_id(authorization_id).is_ok() =>
        {
            (ManagedKind::Phone, Some(authorization_id.clone()))
        }
        [exec, _program, pair, exchange, id]
            if exec == "exec"
                && pair == "pair"
                && exchange == "exchange"
                && marker == format!("{BOOTSTRAP_COMMENT}{id}")
                && valid_id(id).is_ok() =>
        {
            (ManagedKind::Bootstrap, None)
        }
        _ => return None,
    };
    Some(ManagedEntry {
        text,
        command_option: fields[0].start + command_option.start
            ..fields[0].start + command_option.end,
        marker,
        authorization_id,
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

/// Matches only the OpenSSH key type and base64 fields. The trailing comment
/// is deliberately never decoded or parsed: OpenSSH treats it as opaque bytes,
/// so invalid UTF-8 or unmatched quotes there cannot hide a same-key entry.
fn line_contains_public_key(line: &[u8], key: &(String, String)) -> bool {
    fn skip_space(line: &[u8], cursor: &mut usize) {
        while line.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
    }

    fn plain_field<'a>(line: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
        skip_space(line, cursor);
        let start = *cursor;
        while line
            .get(*cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            *cursor += 1;
        }
        (start != *cursor).then_some(&line[start..*cursor])
    }

    let mut cursor = 0;
    skip_space(line, &mut cursor);
    let first_start = cursor;
    let mut quoted = false;
    while let Some(byte) = line.get(cursor).copied() {
        match byte {
            b'\\' if quoted => {
                cursor += 1;
                if cursor == line.len() {
                    return false;
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
    if quoted || first_start == cursor {
        return false;
    }
    let first = &line[first_start..cursor];
    let key_type = key.0.as_bytes();
    let key_data = key.1.as_bytes();
    if first == key_type {
        return plain_field(line, &mut cursor) == Some(key_data);
    }
    plain_field(line, &mut cursor) == Some(key_type)
        && plain_field(line, &mut cursor) == Some(key_data)
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
    // Polling peers must see either no file (or the previous version), or the
    // complete JSON. Never create/truncate the published path before writing.
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    set_mode(staged.path(), 0o600)?;
    staged.write_all(bytes)?;
    staged.as_file().sync_all()?;
    if create_new {
        // Requests are one-use: concurrent exchanges must not replace a winner.
        staged
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
    } else {
        staged.persist(path).map_err(|error| error.error)?;
    }
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
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
    edit_authorized_keys_cleanup(&[&state.bootstrap_marker], None)?;
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
    fn confirmation_requires_an_explicit_affirmative_answer() {
        for answer in ["", "\n", "n\n", "no\n", "maybe\n", "anything else\n"] {
            assert!(
                !read_confirmation(&mut io::Cursor::new(answer)).unwrap(),
                "{answer:?}"
            );
        }
        for answer in ["y\n", "yes\n", " Y \n", "YES\n"] {
            assert!(
                read_confirmation(&mut io::Cursor::new(answer)).unwrap(),
                "{answer:?}"
            );
        }
    }

    // QR enrollment is Unix-only. These tests additionally require POSIX
    // replacement of a pathname while a reader retains the old open inode.
    #[test]
    #[cfg(unix)]
    fn private_files_are_replaced_atomically_and_requests_are_create_once() {
        let directory = isolated();
        let path = directory.path().join("response.json");
        write_json(&path, &serde_json::json!({"approved": false})).unwrap();
        let mut original = fs::File::open(&path).unwrap();
        write_json(&path, &serde_json::json!({"approved": true})).unwrap();
        let mut bytes = Vec::new();
        original.read_to_end(&mut bytes).unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["approved"],
            false
        );
        assert_eq!(
            read_json::<serde_json::Value>(&path).unwrap()["approved"],
            true
        );

        let request = directory.path().join("request.json");
        write_new_json(&request, &serde_json::json!({"id": "first"})).unwrap();
        let error = write_new_json(&request, &serde_json::json!({"id": "second"})).unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().unwrap().kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            read_json::<serde_json::Value>(&request).unwrap()["id"],
            "first"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&request).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn polling_private_files_never_observes_partial_json() {
        use std::sync::{
            Barrier,
            atomic::{AtomicBool, Ordering},
        };

        let directory = isolated();
        let path = directory.path().join("response.json");
        let value = serde_json::json!({"payload": "x".repeat(8 * 1024)});
        write_json(&path, &value).unwrap();
        let started = Barrier::new(2);
        let done = AtomicBool::new(false);
        thread::scope(|scope| {
            let reader = scope.spawn(|| {
                // Always rendezvous before a read assertion can panic.
                started.wait();
                let deadline = Instant::now() + Duration::from_secs(30);
                while !done.load(Ordering::Acquire) {
                    assert!(
                        Instant::now() < deadline,
                        "private-file writer did not finish"
                    );
                    assert_eq!(read_json::<serde_json::Value>(&path).unwrap(), value);
                }
            });
            started.wait();
            let written = (0..64).try_for_each(|_| write_json(&path, &value));
            // Stop/join the reader even when publication failed, before
            // asserting, so a failed write cannot deadlock the scoped thread.
            done.store(true, Ordering::Release);
            reader.join().unwrap();
            written.unwrap();
        });
    }

    #[test]
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn qr_pairing_rejects_unsupported_platforms_before_touching_host_state() {
        assert!(
            pair(&[])
                .unwrap_err()
                .to_string()
                .contains("requires OpenSSH on macOS or Linux")
        );
    }

    #[test]
    fn concurrent_requests_publish_exactly_one_complete_winner() {
        use std::sync::Barrier;

        let directory = isolated();
        let path = directory.path().join("request.json");
        let started = Barrier::new(2);
        let results = thread::scope(|scope| {
            let writers: Vec<_> = (0..2)
                .map(|id| {
                    let path = &path;
                    let started = &started;
                    scope.spawn(move || {
                        started.wait();
                        write_new_json(path, &serde_json::json!({"id": id}))
                    })
                })
                .collect();
            writers
                .into_iter()
                .map(|writer| writer.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let winner = results.iter().position(|result| result.is_ok()).unwrap();
        assert_eq!(read_json::<serde_json::Value>(&path).unwrap()["id"], winner);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
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
    fn gateway_authorization_requires_its_durable_commit_marker() {
        let directory = isolated();
        let marker = directory.path().join("authorization");
        assert!(require_gateway_authorization_at(&marker).is_err());
        fs::write(&marker, b"wrong\n").unwrap();
        assert!(require_gateway_authorization_at(&marker).is_err());
        fs::write(&marker, b"authorized\n").unwrap();
        require_gateway_authorization_at(&marker).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn staged_phone_gateways_include_their_durable_commit_marker() {
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[19; 32]));
        let authorization_id = "44444444444444444444444444444444";
        let (line, phone_marker) = staged_permanent_line(
            "/tmp/pi-desktop-remote",
            private.public_key().clone(),
            authorization_id,
        )
        .unwrap();
        let entry = managed_entry(line_body(line.as_bytes())).unwrap();
        assert_eq!(entry.marker, phone_marker);
        assert_eq!(entry.authorization_id.as_deref(), Some(authorization_id));
        assert!(line.contains("gateway 3"));
        assert!(line.contains(authorization_id));
    }

    #[test]
    fn terminal_qr_forces_standard_black_on_white_colors() {
        let rendered = terminal_qr("pi://pair/v1#test").unwrap();
        assert!(rendered.starts_with("\x1b[30;47m"));
        assert!(rendered.ends_with("\x1b[0m"));
        assert!(rendered.contains('█'));
    }

    #[cfg(unix)]
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn late_writes_to_the_displaced_authorized_keys_remain_reachable() {
        use std::io::{Seek as _, SeekFrom};

        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        fs::write(&keys, b"original\n").unwrap();
        let mut old = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&keys)
            .unwrap();

        edit_authorized_keys_at(keys.clone(), &[], None, Some("pi update\n")).unwrap();
        old.set_len(0).unwrap();
        old.seek(SeekFrom::Start(0)).unwrap();
        old.write_all(b"late external write\n").unwrap();
        old.sync_all().unwrap();

        assert_eq!(fs::read(&keys).unwrap(), b"original\npi update\n");
        let recoveries = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(AUTHORIZED_KEYS_RECOVERY_PREFIX)
            })
            .collect::<Vec<_>>();
        assert_eq!(recoveries.len(), 1);
        assert_eq!(
            fs::read(recoveries[0].path()).unwrap(),
            b"late external write\n"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn authorized_keys_exchange_preserves_a_concurrent_external_edit() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        fs::write(&keys, b"original\n").unwrap();
        let mut proposed = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        proposed.write_all(b"pi update\n").unwrap();
        proposed.as_file().sync_all().unwrap();

        // This is the exact boundary race: another editor replaced the file
        // after Pi derived its proposed contents but before the atomic swap.
        fs::write(&keys, b"external update\n").unwrap();
        assert!(
            !compare_exchange_authorized_keys(proposed.path(), &keys, b"original\n", true,)
                .unwrap()
        );
        assert_eq!(fs::read(&keys).unwrap(), b"external update\n");
        assert_eq!(fs::read(proposed.path()).unwrap(), b"pi update\n");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn authorized_keys_restoration_preserves_a_second_external_edit() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        fs::write(&keys, b"first external update\n").unwrap();
        let mut proposed = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        proposed.write_all(b"pi update\n").unwrap();
        proposed.as_file().sync_all().unwrap();
        let installed = inode_identity(proposed.path()).unwrap();

        // Set up the state immediately after a failed comparison: Pi's update
        // is live and the first external update was displaced for restoration.
        rename_exchange(proposed.path(), &keys).unwrap();
        assert_eq!(fs::read(&keys).unwrap(), b"pi update\n");
        assert_eq!(
            fs::read(proposed.path()).unwrap(),
            b"first external update\n"
        );

        // A second editor replaces Pi's live inode in the restoration window.
        // Restoration must rotate this newest generation back instead of
        // blindly replacing it with the older external update.
        let newest = directory.path().join("newest");
        fs::write(&newest, b"second external update\n").unwrap();
        fs::rename(&newest, &keys).unwrap();
        restore_displaced_authorized_keys(proposed.path(), &keys, installed).unwrap();

        assert_eq!(fs::read(&keys).unwrap(), b"second external update\n");
        assert_eq!(
            fs::read(proposed.path()).unwrap(),
            b"first external update\n"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn provisional_generation_keeps_late_writes_after_restoration() {
        use std::io::{Seek as _, SeekFrom};

        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        fs::write(&keys, b"external update\n").unwrap();
        let mut proposed = tempfile::Builder::new()
            .prefix(AUTHORIZED_KEYS_RECOVERY_PREFIX)
            .tempfile_in(directory.path())
            .unwrap();
        proposed.write_all(b"pi update\n").unwrap();
        proposed.as_file().sync_all().unwrap();
        let installed = inode_identity(proposed.path()).unwrap();
        rename_exchange(proposed.path(), &keys).unwrap();
        let mut late = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&keys)
            .unwrap();

        restore_displaced_authorized_keys(proposed.path(), &keys, installed).unwrap();
        let (recovery, keep_error) = preserve_authorized_keys_recovery(proposed);
        assert!(keep_error.is_none());
        late.set_len(0).unwrap();
        late.seek(SeekFrom::Start(0)).unwrap();
        late.write_all(b"late provisional write\n").unwrap();
        late.sync_all().unwrap();

        assert_eq!(fs::read(&keys).unwrap(), b"external update\n");
        assert_eq!(fs::read(recovery).unwrap(), b"late provisional write\n");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn authorized_keys_restoration_preserves_an_in_place_external_edit() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        fs::write(&keys, b"first external update\n").unwrap();
        let mut proposed = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
        proposed.write_all(b"pi update\n").unwrap();
        proposed.as_file().sync_all().unwrap();
        let installed = inode_identity(proposed.path()).unwrap();
        rename_exchange(proposed.path(), &keys).unwrap();

        // Even an editor that truncates and rewrites Pi's provisional inode,
        // rather than publishing by rename, owns the newest generation.
        fs::write(&keys, b"in-place external update\n").unwrap();
        restore_displaced_authorized_keys(proposed.path(), &keys, installed).unwrap();

        assert_eq!(fs::read(&keys).unwrap(), b"in-place external update\n");
        assert_eq!(
            fs::read(proposed.path()).unwrap(),
            b"first external update\n"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn qr_pairing_replaces_every_entry_with_the_same_phone_key() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[7; 32]));
        let public = private.public_key().clone();
        let openssh = public.to_openssh().unwrap();
        let encoded = openssh.split_whitespace().nth(1).unwrap();
        let (old, old_marker) = permanent_line("/old helper", public.clone()).unwrap();
        let mut original =
            format!("ssh-ed25519 unrelated user@host\n{openssh} manual-copy\n").into_bytes();
        original.extend_from_slice(format!("ssh-ed25519 {encoded} opaque-").as_bytes());
        original.extend_from_slice(b"\xff-comment\n");
        original.extend_from_slice(format!("ssh-ed25519 {encoded} unmatched-\"\n").as_bytes());
        original.extend_from_slice(old.as_bytes());
        fs::write(&keys, original).unwrap();
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn unpublished_phone_authorization_rolls_back_without_losing_other_edits() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[8; 32]));
        let (permanent, _) = permanent_line("/new helper", private.public_key().clone()).unwrap();
        fs::write(&keys, format!("keep\n{permanent}external\n")).unwrap();

        let mut authorization = PhoneAuthorization {
            path: keys.clone(),
            authorization_id: "11111111111111111111111111111111".into(),
            installed: permanent.as_bytes().to_vec(),
            armed: true,
        };
        authorization.rollback().unwrap();

        let text = fs::read_to_string(keys).unwrap();
        assert!(text.contains("keep\n"));
        assert!(text.contains("external\n"));
        assert!(!text.contains("/new helper"));
    }

    #[test]
    fn rollback_never_resurrects_a_concurrently_revoked_phone_key() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[17; 32]));
        let (permanent, _) = permanent_line("/new helper", private.public_key().clone()).unwrap();

        // Pi installed its entry, but a newer external generation revoked every
        // authorization for this key before response publication failed.
        fs::write(&keys, b"keep\nexternal revocation\n").unwrap();
        let mut authorization = PhoneAuthorization {
            path: keys.clone(),
            authorization_id: "22222222222222222222222222222222".into(),
            installed: permanent.as_bytes().to_vec(),
            armed: true,
        };
        authorization.rollback().unwrap();

        assert_eq!(fs::read(&keys).unwrap(), b"keep\nexternal revocation\n");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn rollback_bypasses_a_full_recovery_quota() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[18; 32]));
        let public = private.public_key().clone();
        let (permanent, _) = permanent_line("/new helper", public.clone()).unwrap();
        fs::write(&keys, format!("keep\n{permanent}")).unwrap();
        for index in 0..MAX_AUTHORIZED_KEYS_RECOVERIES {
            fs::write(
                directory
                    .path()
                    .join(format!("{AUTHORIZED_KEYS_RECOVERY_PREFIX}{index}")),
                b"old\n",
            )
            .unwrap();
        }
        assert!(edit_authorized_keys_at(keys.clone(), &[], None, Some("ordinary\n")).is_err());

        let mut authorization = PhoneAuthorization {
            path: keys.clone(),
            authorization_id: "33333333333333333333333333333333".into(),
            installed: permanent.as_bytes().to_vec(),
            armed: true,
        };
        authorization.rollback().unwrap();

        let text = fs::read_to_string(keys).unwrap();
        assert_eq!(text, "keep\n");
        let recovery_count = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(AUTHORIZED_KEYS_RECOVERY_PREFIX)
            })
            .count();
        assert_eq!(recovery_count, MAX_AUTHORIZED_KEYS_RECOVERIES + 1);
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

    #[cfg(unix)]
    #[test]
    fn migration_upgrades_old_phone_gateways_and_ignores_unrecognized_lines() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[9; 32]));
        let (old, marker) =
            permanent_line("/old/hash/pi-desktop-remote", private.public_key().clone()).unwrap();
        // Entries created before gateway v2 are upgraded to the current
        // command while retaining the same restricted key.
        let old = old.replace("gateway 2 ", "gateway ");
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
        assert!(migrated.contains(&format!("exec {} gateway 2 {marker}", stable.display())));
        assert_eq!(migrate_phone_entries_at(keys.clone(), stable).unwrap(), 0);
        assert_eq!(fs::read(&keys).unwrap(), once);
    }

    #[cfg(unix)]
    #[test]
    fn migration_preserves_gateway_v2_terminal_access() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[10; 32]));
        let (old, marker) =
            permanent_line("/old/hash/pi-desktop-remote", private.public_key().clone()).unwrap();
        fs::write(&keys, old).unwrap();
        let stable = Path::new("/home/me/.pi/desktop/bin/pi-desktop-remote");

        assert_eq!(migrate_phone_entries_at(keys.clone(), stable).unwrap(), 1);
        let migrated = fs::read_to_string(keys).unwrap();
        assert!(migrated.contains(&format!("exec {} gateway 2 {marker}", stable.display())));
    }

    #[cfg(unix)]
    #[test]
    fn migration_preserves_gateway_v3_commit_identity() {
        let directory = isolated();
        let keys = directory.path().join("authorized_keys");
        let private = PrivateKey::from(Ed25519Keypair::from_seed(&[20; 32]));
        let authorization_id = "55555555555555555555555555555555";
        let (old, marker) = staged_permanent_line(
            "/old/hash/pi-desktop-remote",
            private.public_key().clone(),
            authorization_id,
        )
        .unwrap();
        fs::write(&keys, old).unwrap();
        let stable = Path::new("/home/me/.pi/desktop/bin/pi-desktop-remote");

        assert_eq!(migrate_phone_entries_at(keys.clone(), stable).unwrap(), 1);
        let migrated = fs::read_to_string(keys).unwrap();
        assert!(migrated.contains(&format!(
            "exec {} gateway 3 {marker} {authorization_id}",
            stable.display()
        )));
    }

    #[test]
    fn stable_pairing_commands_use_the_activated_absolute_path() {
        let stable = Path::new("/home/me/.pi/desktop/bin/pi-desktop-remote");
        let helper = helper_at(stable, true).unwrap();
        assert_eq!(helper.path, stable.to_str().unwrap());
        let key = PrivateKey::from(Ed25519Keypair::from_seed(&[11; 32]));
        let (line, marker) = permanent_line(&helper.path, key.public_key().clone()).unwrap();
        assert!(line.contains(&format!("exec {} gateway 2 {marker}", stable.display())));
        let bootstrap = forced_command(
            &helper.path,
            &["pair", "exchange", "0123456789abcdef0123456789abcdef"],
        )
        .unwrap();
        assert!(bootstrap.contains(&format!("exec {} pair exchange", stable.display())));
    }

    #[test]
    fn the_schedule_gateway_allows_only_the_exact_stdio_operation() {
        assert_eq!(
            gateway_arguments("pi-desktop-remote schedules --stdio").unwrap(),
            vec!["schedules", "--stdio"]
        );
        for rejected in [
            "pi-desktop-remote schedules",
            "pi-desktop-remote schedules --stdio extra",
            "pi-desktop-remote schedules --path /tmp",
            "pi-desktop-remote scheduler start",
            "pi-desktop-remote scheduler stop",
            "pi-desktop-remote scheduler status",
            "pi-desktop-remote scheduler install --confirm",
            "pi-desktop-remote schedules --stdio; touch /tmp/no",
            "pi-desktop-remote schedules --stdio && true",
        ] {
            assert!(gateway_arguments(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn the_forced_gateway_never_allows_activation() {
        assert!(gateway_arguments("pi-desktop-remote sessions").is_ok());
        assert!(gateway_arguments("pi-desktop-remote activate").is_err());
        assert!(gateway_arguments("pi-desktop-remote terminal --stdio").is_ok());
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
