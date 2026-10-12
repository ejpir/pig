//! Explicit local lifecycle. Phone requests never enter this module's install/start paths.
use anyhow::{Context, Result, ensure};
use pi_core::schedules::Health;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
const LABEL: &str = "org.pi-desktop.scheduler";
const UNIT: &str = "pi-desktop-scheduler.service";

pub fn root() -> Result<PathBuf> {
    Ok(std::env::var_os("PI_DESKTOP_REMOTE_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".pi/desktop")))
        .context("No home directory")?
        .join("schedules"))
}
pub fn private_dir(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Scheduler state directory must not be a symlink or special file"
        );
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn lock_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "Scheduler files must be regular files"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}
pub fn locked(root: &Path) -> Result<bool> {
    let lock = lock_file(&root.join("scheduler.lock"))?;
    match lock.try_lock() {
        Ok(()) => Ok(false),
        Err(std::fs::TryLockError::WouldBlock) => Ok(true),
        Err(error) => Err(error.into()),
    }
}
fn installed_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("No home directory")?;
    match std::env::consts::OS {
        "linux" => Ok(dirs::config_dir()
            .unwrap_or_else(|| home.join(".config"))
            .join("systemd/user")
            .join(UNIT)),
        "macos" => Ok(home
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"))),
        _ => {
            anyhow::bail!("Durable scheduling is unsupported on this platform (including Windows)")
        }
    }
}
pub fn health(root: &Path, last_tick_ms: Option<u64>) -> Result<Health> {
    let supported = matches!(std::env::consts::OS, "linux" | "macos");
    let running = supported && locked(root)?;
    #[cfg(not(test))]
    let autostart_installed = supported && installed_path()?.is_file();
    // Unit tests never even inspect a real user's service configuration.
    #[cfg(test)]
    let autostart_installed = false;
    let mut detail: String = if !supported {
        "Windows/this platform: durable scheduling explicitly unsupported".into()
    } else if running {
        "Host scheduler running; host must remain awake. Timeout requests Stop, not a hard boundary.".into()
    } else if autostart_installed {
        "Scheduler service installed but stopped. Run locally: pi-desktop-remote scheduler start"
            .into()
    } else {
        "Host setup required. Run locally: pi-desktop-remote scheduler start; optionally scheduler install --confirm for login autostart. Phone never installs or starts services.".into()
    };
    if supported && !cfg!(feature = "bundled-durable") {
        #[cfg(not(test))]
        let availability = std::env::var_os("PI_DESKTOP_DURABLE_RUNNER")
            .map(PathBuf::from)
            .context("No bundled durable runner or PI_DESKTOP_DURABLE_RUNNER")
            .and_then(|path| {
                Ok(crate::executable::validate(
                    &path,
                    std::env::consts::OS,
                    std::env::consts::ARCH,
                )?)
            });
        #[cfg(test)]
        let availability: Result<()> = Err(anyhow::anyhow!("No fake runner configured"));
        if let Err(error) = availability {
            detail.push_str(&format!(" Durable runner unavailable: {error:#}. Configure a native PI_DESKTOP_DURABLE_RUNNER locally before execution."));
        }
    }
    if running && last_tick_ms.is_some_and(|tick| super::now_ms().saturating_sub(tick) > 120_000) {
        detail.push_str(
            " Scheduler heartbeat is stale; admissions may be blocked on worker recovery.",
        );
    }
    Ok(Health {
        running,
        autostart_installed,
        last_tick_ms,
        detail,
    })
}
pub fn command(args: Vec<String>) -> Result<()> {
    ensure!(
        matches!(std::env::consts::OS, "linux" | "macos"),
        "Windows/this platform: durable scheduling explicitly unsupported"
    );
    let root = root()?;
    private_dir(&root)?;
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["status"] => {
            let store = super::store::Store::open(&root)?;
            println!(
                "{}",
                serde_json::to_string(&health(&root, store.last_tick()?)?)?
            );
        }
        ["serve"] => super::run(&root)?,
        ["start"] => start(&root)?,
        ["stop"] => stop(&root)?,
        ["install", "--confirm"] => install(&root)?,
        _ => anyhow::bail!(
            "Usage: scheduler start | stop | status | install --confirm (opt-in login autostart; does not start now)"
        ),
    }
    Ok(())
}
fn start(root: &Path) -> Result<()> {
    if locked(root)? {
        println!("Scheduler already running");
        return Ok(());
    }
    let installed = installed_path()?;
    if installed.is_file() {
        let status = if cfg!(target_os = "linux") {
            Command::new("systemctl")
                .args(["--user", "start", UNIT])
                .status()?
        } else {
            Command::new("launchctl")
                .args(["bootstrap", &domain()])
                .arg(installed)
                .status()?
        };
        ensure!(
            status.success(),
            "Could not start user scheduler service; inspect the user service manager"
        );
    } else {
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args(["scheduler", "serve"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(lock_file(&root.join("scheduler.log"))?));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid is async-signal-safe and touches no Rust state.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(())
                    }
                });
            }
        }
        let mut child = command.spawn().context("Could not detach scheduler")?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !locked(root)? {
        ensure!(
            Instant::now() < deadline,
            "Scheduler did not start; inspect scheduler.log or the user service manager"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("Scheduler started; no autostart was implicitly installed");
    Ok(())
}
fn stop(root: &Path) -> Result<()> {
    stop_with(
        root,
        installed_path,
        |_| {
            let status = if cfg!(target_os = "linux") {
                Command::new("systemctl")
                    .args(["--user", "stop", UNIT])
                    .status()?
            } else {
                Command::new("launchctl")
                    .args(["bootout", &format!("{}/{LABEL}", domain())])
                    .status()?
            };
            ensure!(status.success(), "Could not stop user scheduler service");
            Ok(())
        },
        Duration::from_secs(10),
    )
}
fn stop_with(
    root: &Path,
    installed: impl FnOnce() -> Result<PathBuf>,
    stop_service: impl FnOnce(&Path) -> Result<()>,
    timeout: Duration,
) -> Result<()> {
    // Installation does not migrate an already detached scheduler into the
    // service manager. Always signal the owner of the caller's state root first,
    // even if service discovery/stopping fails. Never kill a possibly reused PID.
    lock_file(&root.join("stop.request"))?.sync_all()?;
    let service_result = installed().and_then(|path| {
        if path.is_file() {
            stop_service(&path)
        } else {
            Ok(())
        }
    });
    // An inactive unit can report success while a detached root still admits
    // work. Conversely, manager failure must not skip cooperative shutdown.
    if let Err(error) = wait_for_shutdown(root, timeout) {
        return Err(match service_result {
            Ok(()) => error,
            Err(service_error) => error.context(format!(
                "User scheduler service stop also failed: {service_error:#}"
            )),
        });
    }
    service_result.context("Scheduler shutdown verified, but user service stop failed")?;
    println!("Scheduler stopped; existing durable sessions continue.");
    Ok(())
}
fn wait_for_shutdown(root: &Path, timeout: Duration) -> Result<()> {
    let deadline = Instant::now() + timeout;
    while locked(root).context("Could not verify scheduler shutdown")? {
        ensure!(
            Instant::now() < deadline,
            "Scheduler still holds scheduler.lock after stop request; shutdown not verified"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
#[cfg(unix)]
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}
#[cfg(not(unix))]
fn domain() -> String {
    "unsupported".into()
}
fn install(root: &Path) -> Result<()> {
    let path = installed_path()?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let env: Vec<(String, String)> = [
        "PI_DESKTOP_REMOTE_STATE_DIR",
        "PI_DESKTOP_DURABLE_RUNNER",
        "NODE_USE_SYSTEM_CA",
    ]
    .into_iter()
    .filter_map(|key| std::env::var(key).ok().map(|v| (key.into(), v)))
    .collect();
    let template = if cfg!(target_os = "linux") {
        systemd(&executable, &env)?
    } else {
        launchd(&executable, root, &env)?
    };
    // Do not chmod existing systemd/LaunchAgents directories or overwrite an
    // unrelated service. Private data lives only in the scheduler's own root.
    fs::create_dir_all(path.parent().context("Service parent")?)?;
    if path.exists() {
        ensure!(
            fs::read_to_string(&path)?.contains("Pi Desktop host scheduler"),
            "Refusing to overwrite an unrelated service"
        );
    }
    let mut staged = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    staged.write_all(template.as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist(&path)?;
    if cfg!(target_os = "linux") {
        ensure!(
            Command::new("systemctl")
                .args(["--user", "daemon-reload"])
                .status()?
                .success(),
            "Service written, but daemon-reload failed"
        );
        ensure!(
            Command::new("systemctl")
                .args(["--user", "enable", UNIT])
                .status()?
                .success(),
            "Service written, but enable failed"
        );
    }
    println!(
        "Installed login autostart at {} (not started). Run scheduler start explicitly. Linux pre-login operation additionally requires explicit lingering; the host must be awake.",
        path.display()
    );
    Ok(())
}
fn text(path: &Path) -> Result<&str> {
    let value = path.to_str().context("Service paths must be UTF-8")?;
    ensure!(
        !value.chars().any(char::is_control),
        "Control characters in service path"
    );
    Ok(value)
}
fn unit_quote(value: &str, exec: bool) -> Result<String> {
    ensure!(
        !value.chars().any(char::is_control),
        "Control characters in service values"
    );
    let value = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    Ok(format!(
        "\"{}\"",
        if exec {
            value.replace('$', "$$")
        } else {
            value
        }
    ))
}
pub fn systemd(executable: &Path, env: &[(String, String)]) -> Result<String> {
    let mut value = format!(
        "# Pi Desktop host scheduler\n[Unit]\nDescription=Pi Desktop host scheduler\n\n[Service]\nType=simple\nExecStart={} scheduler serve\nRestart=on-failure\nRestartSec=5\nKillMode=process\nUMask=0077\n",
        unit_quote(text(executable)?, true)?
    );
    for (key, content) in env {
        value.push_str(&format!(
            "Environment={}\n",
            unit_quote(&format!("{key}={content}"), false)?
        ));
    }
    value.push_str("\n[Install]\nWantedBy=default.target\n");
    Ok(value)
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
pub fn launchd(executable: &Path, root: &Path, env: &[(String, String)]) -> Result<String> {
    let mut value = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><!-- Pi Desktop host scheduler -->\n<key>Label</key><string>{LABEL}</string>\n<key>ProgramArguments</key><array><string>{}</string><string>scheduler</string><string>serve</string></array>\n<key>RunAtLoad</key><true/><key>AbandonProcessGroup</key><true/><key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n<key>Umask</key><integer>63</integer>\n<key>StandardErrorPath</key><string>{}</string>\n<key>EnvironmentVariables</key><dict>\n",
        xml(text(executable)?),
        xml(text(&root.join("scheduler.log"))?)
    );
    for (key, content) in env {
        ensure!(
            !content.chars().any(char::is_control),
            "Control characters in service values"
        );
        value.push_str(&format!(
            "<key>{}</key><string>{}</string>\n",
            xml(key),
            xml(content)
        ));
    }
    value.push_str("</dict></dict></plist>\n");
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn templates_escape_paths_without_shell_execution_or_implicit_start() {
        let path = Path::new("/tmp/Pi $HOME %i \\\"<&' helper");
        let unit = systemd(
            path,
            &[(
                "PI_DESKTOP_REMOTE_STATE_DIR".into(),
                "/tmp/state %h \"x\"".into(),
            )],
        )
        .unwrap();
        assert!(unit.contains("$$HOME %%i"));
        assert!(unit.contains("\\\\\\\""));
        assert!(!unit.contains("/bin/sh"));
        assert!(!unit.contains("--now"));
        let plist = launchd(path, Path::new("/tmp/private"), &[]).unwrap();
        assert!(plist.contains("&lt;&amp;&apos;"));
        assert!(plist.contains("&quot;"));
        assert!(systemd(Path::new("/tmp/a\nb"), &[]).is_err());
    }
    fn stop_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("caller-state/schedules");
        private_dir(&root).unwrap();
        let installed = temp.path().join("fake-user-services").join(UNIT);
        fs::create_dir_all(installed.parent().unwrap()).unwrap();
        fs::write(
            &installed,
            systemd(Path::new("/fake/pi desktop"), &[]).unwrap(),
        )
        .unwrap();
        (temp, root, installed)
    }
    #[test]
    fn installed_service_success_cannot_hide_a_detached_scheduler() {
        let (_temp, root, installed) = stop_fixture();
        let owner = lock_file(&root.join("scheduler.lock")).unwrap();
        owner.try_lock().unwrap();
        let error = stop_with(
            &root,
            || Ok(installed.clone()),
            |path| {
                assert_eq!(path, installed);
                assert!(root.join("stop.request").is_file());
                assert!(locked(&root).unwrap());
                Ok(()) // An installed but inactive unit reports success.
            },
            Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("shutdown not verified"));
        assert!(locked(&root).unwrap());
        assert!(root.join("stop.request").is_file());
    }
    fn detached_stop_waits_for_release(service_fails: bool) {
        use std::sync::mpsc::{self, RecvTimeoutError};
        let (temp, root, installed) = stop_fixture();
        let owner = lock_file(&root.join("scheduler.lock")).unwrap();
        owner.try_lock().unwrap();
        let other_root = temp.path().join("other-state/schedules");
        private_dir(&other_root).unwrap();
        let other_owner = lock_file(&other_root.join("scheduler.lock")).unwrap();
        other_owner.try_lock().unwrap();
        let (manager_called, manager_response) = mpsc::channel();
        let (stopped, stop_response) = mpsc::channel();
        std::thread::scope(|threads| {
            threads.spawn(|| {
                let result = stop_with(
                    &root,
                    || Ok(installed.clone()),
                    |path| {
                        assert_eq!(path, installed);
                        assert!(root.join("stop.request").is_file());
                        assert!(locked(&root).unwrap());
                        manager_called.send(()).unwrap();
                        if service_fails {
                            anyhow::bail!("fake service-manager failure");
                        }
                        Ok(())
                    },
                    Duration::from_secs(5),
                );
                stopped.send(result).unwrap();
            });
            manager_response
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            // This synthetic detached owner has received the cooperative signal,
            // but has not exited. Neither manager success nor failure may return
            // before actual lock release (or an explicit verification timeout).
            assert!(root.join("stop.request").is_file());
            assert!(matches!(
                stop_response.recv_timeout(Duration::from_millis(50)),
                Err(RecvTimeoutError::Timeout)
            ));
            drop(owner);
            let result = stop_response.recv_timeout(Duration::from_secs(5)).unwrap();
            if service_fails {
                let error = format!("{:#}", result.unwrap_err());
                assert!(error.contains("shutdown verified"));
                assert!(error.contains("fake service-manager failure"));
            } else {
                result.unwrap();
            }
        });
        assert!(!locked(&root).unwrap());
        assert!(locked(&other_root).unwrap());
        assert!(!other_root.join("stop.request").exists());
    }
    #[test]
    fn installed_stop_signals_detached_root_and_verifies_shutdown() {
        detached_stop_waits_for_release(false);
    }
    #[test]
    fn service_failure_still_signals_detached_root_and_verifies_shutdown() {
        detached_stop_waits_for_release(true);
    }
    #[test]
    fn service_failure_and_held_root_report_unverified_shutdown() {
        let (_temp, root, installed) = stop_fixture();
        let owner = lock_file(&root.join("scheduler.lock")).unwrap();
        owner.try_lock().unwrap();
        let error = stop_with(
            &root,
            || Ok(installed),
            |_| anyhow::bail!("fake service-manager failure"),
            Duration::ZERO,
        )
        .unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("fake service-manager failure"));
        assert!(error.contains("shutdown not verified"));
        assert!(root.join("stop.request").is_file());
        assert!(locked(&root).unwrap());
    }
    #[test]
    fn stop_without_installation_does_not_call_service_manager_or_install() {
        let (_temp, root, installed) = stop_fixture();
        fs::remove_file(&installed).unwrap();
        stop_with(
            &root,
            || Ok(installed.clone()),
            |_| panic!("No service manager should be called"),
            Duration::ZERO,
        )
        .unwrap();
        assert!(root.join("stop.request").is_file());
        assert!(!locked(&root).unwrap());
        assert!(!installed.exists());
    }
    #[cfg(unix)]
    #[test]
    fn stop_rejects_symlink_request_without_touching_service_manager() {
        use std::os::unix::{fs::PermissionsExt, fs::symlink};
        let (temp, root, _installed) = stop_fixture();
        let unrelated = temp.path().join("unrelated");
        fs::write(&unrelated, "do not modify").unwrap();
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&unrelated, root.join("stop.request")).unwrap();
        assert!(
            stop_with(
                &root,
                || panic!("Service discovery should not run after an unsafe signal"),
                |_| panic!("No service manager should be called"),
                Duration::ZERO,
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&unrelated).unwrap(), "do not modify");
        assert_eq!(
            fs::metadata(&unrelated).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    #[test]
    fn exclusive_lock_and_private_state() {
        let temp = tempfile::tempdir().unwrap();
        private_dir(temp.path()).unwrap();
        let first = lock_file(&temp.path().join("scheduler.lock")).unwrap();
        first.try_lock().unwrap();
        assert!(locked(temp.path()).unwrap());
        drop(first);
        assert!(!locked(temp.path()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }
}
