#!/usr/bin/env python3
"""Run Android's live transport against a disposable, loopback-only SSH server.

Requires OpenSSH's sshd/ssh-keygen, the built pi-desktop-remote helper, and the
standalone faux durable fixture. Never modifies the account's authorized_keys,
normal SSH service, provider configuration, home directory, or real sessions.
"""
import argparse
import getpass
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time


def forced_command(config_path):
    config = json.loads(Path(config_path).read_text())
    command = shlex.split(os.environ.get("SSH_ORIGINAL_COMMAND", ""))
    helper = config["helper"]
    environment = os.environ.copy()
    environment.update({
        "PI_DESKTOP_REMOTE_STATE_DIR": config["state"],
        "PI_DESKTOP_DURABLE_RUNNER": config["runner"],
        "PI_DESKTOP_DURABLE_PROVIDER": "faux",
        "PI_DESKTOP_DURABLE_MODEL": "faux-1",
        "PI_DESKTOP_PI": str(Path(config["state"]) / "DO-NOT-START-STOCK-PI"),
    })
    environment.pop("PI_DESKTOP_RPC_ENTRY", None)
    # Discovery normally searches the account's helper installations. This
    # test endpoint exposes exactly the selected build, without changing HOME.
    if len(command) == 3 and command[:2] == ["sh", "-c"] and command[2] == config["find"]:
        version = subprocess.check_output([helper, "--version"], env=environment, text=True).strip()
        capabilities = subprocess.check_output([helper, "--capabilities"], env=environment, text=True).strip()
        print(f"home\t{config['project']}")
        print(f"{helper}\t{version}\t{capabilities}")
        return
    arguments = command[1:]
    allowed = arguments in [["sessions"], ["models"], ["connect", "--stdio"]]
    if len(arguments) == 3 and arguments[:2] == ["directories", "--path"]:
        allowed = Path(arguments[2]).resolve().is_relative_to(Path(config["project"]))
    if not command or command[0] != helper or not allowed:
        raise SystemExit("Only the isolated helper's test commands are allowed")
    os.execve(helper, command, environment)


def terminate_test_daemons(directory):
    # Detached daemons are not children of sshd. Match our unique, generated
    # project/state path AND the helper/fixture executable before signalling.
    processes = subprocess.check_output(["ps", "-axo", "pid=,command="], text=True)
    for line in processes.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) != 2:
            continue
        pid, command = fields
        if str(directory) not in command or not any(name in command for name in ("pi-desktop-remote", "pi-desktop-durable-fixture")):
            continue
        if int(pid) == os.getpid():
            continue
        try:
            os.kill(int(pid), signal.SIGTERM)
        except ProcessLookupError:
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--sshd", default=shutil.which("sshd") or "/usr/sbin/sshd")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for executable in (args.helper, args.runner):
        if not executable.is_file() or not os.access(executable, os.X_OK):
            parser.error(f"Build the standalone executable first: {executable}")
    if "fixture" not in args.runner.name:
        parser.error("The runner must be the faux-only fixture, never a real provider runner")
    repository = Path(__file__).resolve().parents[3]
    source = (repository / "crates/pi_android/src/remote.rs").read_text()
    discovery = source.split('const FIND: &str = r#"', 1)[1].split('"#;', 1)[0]
    with tempfile.TemporaryDirectory(prefix="pi-ssh-regression-") as temporary:
        directory = Path(temporary).resolve()
        project = directory / "project"
        project.mkdir()
        state = directory / "state"
        state.mkdir()
        keys = directory / "authorized_keys"
        keys.touch(mode=0o600)
        host_key = directory / "host_key"
        subprocess.run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(host_key)], check=True)
        config = directory / "fixture.json"
        config.write_text(json.dumps({
            "helper": str(args.helper.resolve()), "runner": str(args.runner.resolve()),
            "state": str(state), "project": str(project), "find": discovery,
        }))
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        ssh_config = directory / "sshd_config"
        force = shlex.join([sys.executable, str(Path(__file__).resolve()), "--forced-command", str(config)])
        ssh_config.write_text("\n".join([
            f"Port {port}", "ListenAddress 127.0.0.1", f'HostKey "{host_key}"',
            f'PidFile "{directory / "sshd.pid"}"', f'AuthorizedKeysFile "{keys}"',
            "StrictModes yes", "PasswordAuthentication no", "KbdInteractiveAuthentication no",
            "PubkeyAuthentication yes", "UsePAM no", "PermitRootLogin no",
            f"AllowUsers {getpass.getuser()}", "AllowTcpForwarding no", "AllowAgentForwarding no",
            "X11Forwarding no", "PermitTTY no", f"ForceCommand {force}", "LogLevel VERBOSE", "",
        ]))
        subprocess.run([args.sshd, "-t", "-f", str(ssh_config)], check=True)
        with (args.output / "sshd.log").open("wb") as log:
            server = subprocess.Popen([args.sshd, "-D", "-e", "-f", str(ssh_config)], stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 8
                while True:
                    if server.poll() is not None:
                        raise RuntimeError(f"Test sshd exited; see {args.output / 'sshd.log'}")
                    try:
                        with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                            break
                    except OSError:
                        if time.monotonic() > deadline:
                            raise RuntimeError("Test sshd did not listen on loopback")
                        time.sleep(0.1)
                environment = os.environ.copy()
                environment.update({
                    "PI_ANDROID_TEST_SSH": f"{getpass.getuser()}@127.0.0.1:{port}",
                    "PI_ANDROID_TEST_KEYS": str(keys), "PI_ANDROID_TEST_PROJECT": str(project),
                })
                result = subprocess.run([
                    "cargo", "test", "-p", "pi_android", "--lib", "--offline",
                    "a_durable_session_runs_over_ssh", "--", "--ignored", "--nocapture",
                ], cwd=repository, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
                (args.output / "test.log").write_bytes(result.stdout)
                sys.stdout.buffer.write(result.stdout)
                result.check_returncode()
                print("PASS isolated SSH catalog, folders, image prompt, follow-ups, second watcher and deletion")
            finally:
                server.terminate()
                server.wait(timeout=5)
                terminate_test_daemons(directory)


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--forced-command":
        forced_command(sys.argv[2])
    else:
        main()
