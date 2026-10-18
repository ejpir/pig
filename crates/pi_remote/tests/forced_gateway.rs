use std::{
    io::{Read as _, Write as _},
    process::{Command, Stdio},
};

const MARKER: &str = "pi-phone:test-device";

fn gateway(versioned: bool, original: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pi-desktop-remote"));
    command.arg("gateway");
    if versioned {
        command.arg("2");
    }
    command.arg(MARKER).env("SSH_ORIGINAL_COMMAND", original);
    command
}

#[test]
fn legacy_and_current_gateways_dispatch_an_ordinary_capability() {
    for versioned in [false, true] {
        let output = gateway(versioned, "pi-desktop-remote --capabilities")
            .output()
            .expect("run forced gateway");
        assert!(
            output.status.success(),
            "gateway stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let capabilities: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("capability JSON");
        assert_eq!(capabilities["sessions"], true);
        assert_eq!(
            capabilities["fileEditing"],
            cfg!(any(target_os = "linux", target_os = "macos"))
        );
        assert_eq!(capabilities["terminal"], cfg!(unix));
    }
}

#[cfg(unix)]
#[test]
fn legacy_and_current_gateways_open_a_terminal() {
    // Open protocol v1 at 80x24 in the account's home directory, then close.
    let input = [
        1, 0, 0, 0, 6, // Open and payload length.
        0, 1, // Protocol version.
        0, 80, 0, 24, // Dimensions.
        4, 0, 0, 0, 0, // Close.
    ];
    for versioned in [false, true] {
        let mut child = gateway(versioned, "pi-desktop-remote terminal --stdio")
            .env("SHELL", "/bin/sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn terminal gateway");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&input)
            .expect("write terminal frames");
        let output = child.wait_with_output().expect("wait for terminal gateway");
        assert!(
            output.status.success(),
            "gateway stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stdout.starts_with(&[0x81, 0, 0, 0, 0]),
            "terminal did not emit Opened: {:?}",
            output.stdout
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn terminal_close_kills_a_reparented_setsid_descendant() {
    use pi_core::terminal_transport::{ClientMessage, Dimensions, ServerDecoder, ServerMessage};
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    let mut child = gateway(true, "pi-desktop-remote terminal --stdio")
        .env("SHELL", "/bin/sh")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn terminal gateway");
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            &ClientMessage::Open {
                dimensions: Dimensions::new(80, 24).unwrap(),
                cwd: String::new(),
            }
            .encode()
            .unwrap(),
        )
        .unwrap();
    stdin
        .write_all(
            &ClientMessage::Input(
                b"/usr/bin/setsid -f /bin/sh -c 'echo DAEMON:$$; trap \"\" HUP TERM; while :; do sleep 1; done'\n"
                    .to_vec(),
            )
            .encode()
            .unwrap(),
        )
        .unwrap();

    let mut stdout = child.stdout.take().unwrap();
    let (pid_tx, pid_rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut decoder = ServerDecoder::new();
        let mut bytes = [0u8; 4096];
        let mut terminal = Vec::new();
        let mut sent = false;
        while let Ok(read) = stdout.read(&mut bytes) {
            if read == 0 {
                break;
            }
            for message in decoder.push(&bytes[..read]).unwrap() {
                if let ServerMessage::Output(output) = message {
                    terminal.extend_from_slice(&output);
                    if let Some(marker) = terminal
                        .windows(b"DAEMON:".len())
                        .rposition(|window| window == b"DAEMON:")
                    {
                        let digits = terminal[marker + b"DAEMON:".len()..]
                            .iter()
                            .copied()
                            .take_while(u8::is_ascii_digit)
                            .collect::<Vec<_>>();
                        if !sent && !digits.is_empty() {
                            let pid = std::str::from_utf8(&digits)
                                .unwrap()
                                .parse::<i32>()
                                .unwrap();
                            let _ = pid_tx.send(pid);
                            sent = true;
                        }
                    }
                }
            }
        }
    });
    let daemon = pid_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("daemon pid from PTY");
    stdin
        .write_all(&ClientMessage::Close.encode().unwrap())
        .unwrap();
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "terminal helper did not stop");
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
    let _ = reader.join();

    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline && unsafe { libc::kill(daemon, 0) } == 0 {
        thread::sleep(Duration::from_millis(10));
    }
    assert_ne!(
        unsafe { libc::kill(daemon, 0) },
        0,
        "setsid descendant survived terminal close"
    );
}

#[test]
fn forced_gateways_still_reject_local_only_commands() {
    for versioned in [false, true] {
        let output = gateway(versioned, "pi-desktop-remote activate")
            .output()
            .expect("run rejected forced command");
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("This phone key cannot run that command")
        );
    }
}
