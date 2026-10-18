# Validation

Recorded validation includes Linux aarch64, Linux x86_64 and an Android x86_64 emulator, using isolated projects and configuration. Local component tests and native UI checks do not establish successful hosted releases, macOS/Windows behavior or live provider operation.

## Recorded local coverage

| Area | Evidence | Scope |
| --- | --- | --- |
| Rust | Workspace regression tests; separate fake-LSP tests; warnings-denied Clippy | Session lifecycle, transport, rendering isolation, input, editor and jj behavior |
| Desktop extension | Channel unit tests; pi_core's transport test and a native Xvfb capture against Pi's 1.3.0 release binary | Routing, `hello` and reconnection, metadata commands, active tools, settings, sessions, trust, tree navigation, reload and fork into a folder |
| Embedded pi | Desktop unpack tests; `fetch_pi.py` digest, layout and version checks | Cache extraction of the embedded archive |
| Packaging | Offline archive and release-policy tests | Binary architecture, staging, notices, exact target matrix, checksums and tag validation |
| Native UI | Capture scripts driving real GPUI windows and the X11 clipboard | Layout, selection/copy, scrolling, dialogs, navigation and file operations |

These checks were recorded in separate runs, not one combined release qualification. Mocked signing and uploads verify control flow only. Screenshots under `design/` are visual references, not execution evidence.

## Reproduce the checks

Install the [build prerequisites](../README.md#build-and-run) and use the sibling Zed checkout pinned in [the workflow](../.github/workflows/desktop.yml). CI pins Rust 1.98.1, Python 3.12, Node 22.19.0 and Bun 1.4.3 (npm collects bundled dependency notices when packaging).

From the repository root:

```sh
# Linux: keep these outputs separate from another host's native artifacts.
export CARGO_TARGET_DIR=target/linux
export CARGO_INCREMENTAL=0

# Rebuild ignored Local bundles; their manifests make Cargo reject stale inputs.
(cd crates/pi_local && npm ci --ignore-scripts && bun run build)
(cd backend/durable && npm ci --ignore-scripts)
python3 experiments/bun-embedding/build_local.py
git diff --exit-code  # generated work must not drift tracked inputs or lockfiles

# Format only this repository, not the sibling Zed checkout.
cargo fmt -p pi_visual -p pi_visual_ui -p pi_markdown -p pi_core -p pi_remote \
  -p pi_android -p gpui_android -p pi_editor -p pi_jj -p pi_lsp_bridge \
  -p pi_settings -p terminal -p pi_terminal -p pi_terminal_mobile -p pi-desktop -- --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets --no-deps -- -D warnings
cargo install cargo-about --version 0.9.2 --locked --features cli
python scripts/rust_notices.py --check

# Zed test-support features must run separately from workspace tests.
cargo test --locked -p pi_lsp_bridge --features fake-lsp
cargo clippy --locked -p pi_lsp_bridge --features fake-lsp \
  --all-targets --no-deps -- -D warnings

python3 -m unittest discover -s scripts -p 'test_packaging.py' -v
```

To test the desktop extension against the embedded pi on Linux arm64 (no provider calls):

```sh
python3 scripts/fetch_pi.py --platform linux-arm64 --out "$PWD/artifacts/pi"
PI_DESKTOP_TEST_PI="$PWD/artifacts/pi/linux-arm64/pi" \
  cargo test --locked -p pi_core --test transport -- --ignored

export PI_DESKTOP_BACKEND_ARCHIVE="$PWD/artifacts/pi/pi-linux-arm64.tar.gz"
cargo test --locked -p pi-desktop --features bundled-backend
cargo clippy --locked -p pi-desktop --features bundled-backend \
  --all-targets --no-deps -- -D warnings
```

## Native UI checks

Build the desktop before captures. Linux probes use Xvfb and software Vulkan; they are not browser or SVG renders. Clipboard checks require the real `/usr/bin/xclip`, not a host-clipboard shim.

CI capture scripts isolate desktop preferences as well as HOME/XDG state. The first-screen probe explicitly starts in Evening, then toggles to Moonstone; it does not depend on the OS theme. OCR uses enlarged copies, while palette and geometry checks retain the original pixels. Tool headers are located from rendered text, and the new-session probe completes the chooser before checking the landing page. Details are closed on first launch and captured separately after an explicit request. `compare-thread-study.py` now compares the approved workbench proposal's warm surfaces and fluid composer gutters, not the superseded inspector-first study. Different viewport sizes, transcript content, status colors and neutral borders are disclosed rather than asserted as pixel-identical.

```sh
cargo build --locked -p pi-desktop
xvfb-run -a -s '-screen 0 1440x900x24' bash scripts/capture-first-screen.sh
python3 scripts/validate-screens.py
python3 scripts/compare-thread-study.py
```

Run additional scripts with the same Xvfb invocation:

| Script in `scripts/` | Checks |
| --- | --- |
| `capture-workbench.py --binary /path/to/pi-desktop` | Thread light/dark/wide, changed files, split review, explicit revision attachment, compact unified fallback and clean quit; labelled demo data, no sample checks executed |
| `capture-tool-selection.sh` | Native mouse selection and keyboard copy from expanded edit detail |
| `capture-session-close.sh` | Cancel-default close, process cleanup and retained projects/conversations |
| `capture-file-actions.sh` | No overwrite, containment, dirty-buffer guards, rename and Trash |
| `capture-diagnostic-hover.sh` | Deterministic local LSP diagnostics, quick fix and prompt handoff |
| `capture-composer-scrollbar.sh` | Wrapped input, drag/track/wheel scrolling and unchanged draft copy |
| `capture-projects-shell.sh` | Chooser, model picker, resources, directory references and correlated dialogs |
| `capture-status-tools.sh` | Session-local TPS display and names-only tools with metadata tooltips |
| `capture-process-scrollbar.sh` | Demo overflow, thumb/track input and fixed popup header/footer |
| `capture-backend.sh` | Desktop interoperability with Pi's release binary and the desktop extension; metadata-only command checks |

Capture scripts isolate HOME, XDG directories, agent configuration and project files. Scripts with real pi use `PI_DESKTOP_TEST_PI`, or fetch Pi's release binary into `artifacts/pi`, and log both stdio and the desktop channel through `scripts/pi-proxy.mjs`. Scripts with fake peers answer the channel through `fixtures/desktop_channel.py`. Historical tools are displayed, not replayed. The shell probe executes a harmless local fixture command; the diagnostic probe sends a prompt to a fake backend that rejects model execution. The process-scrollbar probe starts no Pi subprocesses. Pixel/OCR comparisons check specific geometry and content, not complete pixel equality or hardware frame rate.

Logs, screenshots and protocol proofs are local, ignored outputs under `artifacts/`. Diagnostics apply best-effort redaction only; review them before sharing.

## Android scheduling checks (2026-10-11)

The approved [schedule study](../design/android/schedules/index.html) is implemented
in native GPUI Android screens with a separate host-owned scheduler. See
[host setup and execution policy](../crates/pi_remote/README.schedules.md) and the
[native verification instructions](../crates/pi_android/scripts/SCHEDULER_VERIFICATION.md).

Recorded local evidence is ignored under `artifacts/schedules-integration/` and
`artifacts/schedules-native-20261011T144203Z/`. Android `ui-test` tests passed
(**279 passed, two existing SSH/pairing tests ignored**); the Python harness
safety suite passed all **14** tests. Warnings-denied workspace and Android
`ui-test` Clippy passed. The x86_64 UI-test APK and production arm64 APK built;
only the x86_64 UI-test APK was installed and exercised.

Native verification used **one** API-35 emulator (`emulator-5554` / `pi_api35`),
not two emulators: 720×1280 at 320 dpi and 1080×2400 at 420 dpi. Both runs passed
create/edit/preview/consent, pause/resume, confirmed Run now/delete, distinct
admission/history outcomes, linked-session Back navigation, offline/setup/
unsupported guards, and light/dark rendering. Screenshots were visually
inspected. Preferences remained byte-for-byte identical and physical display
settings were restored. These inert sample flows prove UI behavior, **not**
real SSH/provider scheduling. No real credentials, jobs or service installation
were used.

Reproduce the additional checks:

```sh
cargo test --locked -p pi_core -p pi_remote
cargo test --locked -p pi_android --features ui-test
cargo clippy --locked -p pi_android --features ui-test --all-targets --no-deps -- -D warnings
python3 -m unittest discover -s crates/pi_android/scripts -p 'test_schedules_tooling.py' -v
# Explicitly built faux-only fixture; never substitute the production runner.
PI_DESKTOP_TEST_DURABLE_RUNNER="$PWD/artifacts/durable/pi-desktop-durable-fixture" \
  cargo test --locked -p pi_remote --test durable --test schedules -- --ignored
```

Current terminal coverage includes binary frame fragmentation and limits, bounded queue overflow, external PTY cleanup, retained Android views, one-shot pinch geometry, IME replacement and process-tree termination with a SIGHUP-ignoring descendant. Actual systemd/launchd installation, macOS scheduling and live-provider runs remain unverified; synthetic manager tests and faux runtime tests are not substitutes for those checks.

## CI and publication

The workflow runs checks on Linux amd64 and builds native packages for:

| Platform | Target | Package |
| --- | --- | --- |
| Linux amd64 | `x86_64-unknown-linux-gnu` | `pi-desktop-linux-amd64.tar.gz` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `pi-desktop-linux-arm64.tar.gz` |
| macOS arm64 | `aarch64-apple-darwin` | `pi-desktop-macos-arm64.zip` |
| Windows amd64 | `x86_64-pc-windows-msvc` | `pi-desktop-windows-amd64.zip` |

Pushes, pull requests and manual runs upload package artifacts. Version tags publish the four desktop archives, four helpers, Android APK, Android Local runtime and relink archives, and `SHA256SUMS` only after checks and native builds succeed. Missing or unexpected files fail validation; published assets are not overwritten. Validation artifacts are separate from release packages.

## Unverified behavior

- Hosted release builds and publication, native macOS/Windows UI and embedded-pi execution; the channel's Windows TCP path is tested on Linux only.
- Developer ID signing/notarization and Windows signing. Current macOS packages are ad-hoc signed; Windows packages are unsigned.
- Live provider streaming, real authentication, remote package operations and sharing uploads. Upload tests use mocks and synthetic conversations.
- Broad production language-server compatibility, native add-ons under Bun and large-repository snapshot costs.

Offline mode is not a sandbox: explicitly requested tools or extensions can still execute. Validation must not use real credentials, conversations or projects. Cross-compilation, mocked commands and a configured workflow are not substitutes for native execution or a successful hosted run.
