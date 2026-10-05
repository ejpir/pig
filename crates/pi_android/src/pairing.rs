//! QR enrollment is transport code, independent of GPUI. The QR contains a
//! one-use SSH identity and pinned host fingerprints; successful local approval
//! replaces it with the phone's lasting key behind a restricted command gateway.

use crate::{
    remote,
    ssh::{Address, Connection, Identity},
};
use anyhow::Context as _;

pub async fn enroll(
    offer: pi_core::pairing::Offer,
    identity: Identity,
    public_key: String,
    device_name: String,
) -> anyhow::Result<(Address, Connection, remote::Helper, Vec<remote::Listed>)> {
    let bootstrap = Identity::from_pairing_seed(&offer.seed()?);
    let trusted = Some(offer.host_keys.clone());
    let mut last_error = None;
    let mut opened = None;
    for host in &offer.hosts {
        let address = Address {
            user: offer.user.clone(),
            host: host.clone(),
            port: offer.port,
        };
        match Connection::open_with_host_keys(address.clone(), bootstrap.clone(), trusted.clone())
            .await
        {
            Ok(connection) => {
                opened = Some((address, connection));
                break;
            }
            Err(error) => last_error = Some(error),
        }
    }
    let (address, bootstrap_connection) = opened.ok_or_else(|| {
        last_error.unwrap_or_else(|| anyhow::anyhow!("The QR has no reachable computer address"))
    })?;
    let request = pi_core::pairing::Request::new(offer.id, public_key, device_name);
    let pipe = bootstrap_connection
        .pipe("pi-desktop-remote pair exchange".into())
        .await?;
    pipe.input.send(serde_json::to_value(request)?).await?;
    let record = match pipe.records.recv().await {
        Ok(record) => record,
        Err(_) => {
            let reason = pipe
                .ended
                .recv()
                .await
                .unwrap_or_else(|_| "The pairing connection closed".into());
            anyhow::bail!("Pairing failed: {reason}");
        }
    };
    let response: pi_core::pairing::Response = serde_json::from_value(record)?;
    if response.version != pi_core::pairing::VERSION {
        anyhow::bail!("The computer returned an unsupported pairing response");
    }
    if !response.approved {
        anyhow::bail!(
            "{}",
            response
                .error
                .as_deref()
                .unwrap_or("Pairing was denied on the computer")
        );
    }
    let paired = response
        .helper
        .context("The computer didn't identify its remote helper")?;
    if !paired.gateway || paired.path.is_empty() || paired.home.is_empty() {
        anyhow::bail!("The computer did not install a restricted Pi gateway");
    }
    drop(pipe);
    drop(bootstrap_connection);

    let connection = Connection::open_with_host_keys(address.clone(), identity, trusted).await?;
    let helper = remote::Helper {
        path: paired.path,
        home: paired.home,
        images: paired.images,
        gateway: true,
    };
    let listed = remote::sessions(&connection, &helper).await?;
    Ok((address, connection, helper, listed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead as _, BufReader},
        path::Path,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    fn variable(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
    }

    /// Runs against scripts/test_ssh.py's disposable loopback sshd. The helper
    /// creates and replaces a real forced-command bootstrap entry, but only in
    /// the generated authorized_keys file guarded below.
    #[test]
    #[ignore = "needs scripts/test_ssh.py's disposable SSH server"]
    fn qr_pairing_replaces_the_bootstrap_with_a_restricted_phone_key() {
        let address = Address::parse(&variable("PI_ANDROID_TEST_SSH")).unwrap();
        let keys_value = variable("PI_ANDROID_TEST_KEYS");
        let keys = Path::new(&keys_value).canonicalize().unwrap();
        let directory = keys.parent().unwrap();
        assert!(
            directory
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("pi-ssh-regression-"),
            "Use scripts/test_ssh.py; never an account's real authorized_keys"
        );
        assert!(std::fs::read(&keys).unwrap().is_empty());
        let pairing_dir = directory.join("pairing");
        let helper = variable("PI_ANDROID_TEST_HELPER");
        let port = address.port.to_string();
        let mut process = Command::new(&helper)
            .args([
                "pair",
                "--host",
                "127.0.0.1",
                "--port",
                &port,
                "--expires",
                "60",
                "--json",
                "--yes",
            ])
            .env("PI_DESKTOP_AUTHORIZED_KEYS_FILE", &keys)
            .env("PI_DESKTOP_PAIRING_DIR", &pairing_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdout = process.stdout.take().unwrap();
        let mut output = BufReader::new(stdout);
        let mut url = String::new();
        output.read_line(&mut url).unwrap();
        let offer = pi_core::pairing::Offer::parse(url.trim()).unwrap();
        let identity = Identity::load_or_create(&directory.join("paired/id_ed25519")).unwrap();
        let public_key = identity.public_line();
        let expected_code = pi_core::pairing::confirmation_code(&offer.id, &public_key);

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (paired_address, connection, paired_helper, sessions) = runtime
            .block_on(enroll(
                offer,
                identity,
                public_key,
                "Regression phone".into(),
            ))
            .unwrap();
        assert_eq!(paired_address, address);
        assert!(paired_helper.gateway);
        assert!(sessions.is_empty());
        let denied = runtime
            .block_on(connection.run("sh -c 'id'".into()))
            .unwrap();
        assert_ne!(
            denied.status,
            Some(0),
            "paired keys must not expose a shell"
        );

        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = process.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "pair helper did not finish");
            std::thread::sleep(Duration::from_millis(50));
        };
        assert!(status.success());
        let remaining: String = output
            .lines()
            .map(Result::unwrap)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(remaining.contains(&expected_code));
        let authorized = std::fs::read_to_string(&keys).unwrap();
        assert!(authorized.contains("pi-phone:"));
        assert!(!authorized.contains("pi-pair-bootstrap:"));
        assert!(authorized.contains(" gateway "));
    }
}
