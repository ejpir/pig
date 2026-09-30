# Pi Desktop

A native desktop app for the [pi](https://github.com/earendil-works/pi) coding agent, built with Zed's GPUI (no webview). Each session runs its own pi process in its project folder.

- **Sessions and projects:** a sidebar of open and saved sessions per project, with search, forks, worktrees and **All Sessions**.
- **Thread:** Markdown messages, collapsible tool calls, a `/` command menu, `@` mentions, attachments, and model and thinking pickers.
- **jj turns:** each run that changes files becomes a jj change you can undo, redo or restore file by file (opt-in per project).
- **Language servers:** file tabs on Zed's editor. Errors go back to pi after its edits, even though pi has no LSP support itself.
- **Terminal drawer**, **Settings** (pi's settings and the app's own), **Models** and **Resources**.

See [docs/architecture.md](docs/architecture.md) for how it works.

## Install

Download a package for Linux (amd64, arm64), macOS (Apple Silicon) or Windows (amd64) from the releases. pi's backend is built into the executable, so you don't need Node.js or pi. Existing pi settings and sign-ins in `~/.pi/agent` are reused. Signing in to a provider currently opens a terminal that runs `pi`, so that one step still needs pi installed.

The macOS app is ad-hoc signed and the Windows ZIP is unsigned, so the first launch may need an explicit override.

## Build from source

Keep a checkout of Zed next to this repository:

```text
repos/
  pi-desktop/
  zed/   # revision 5becf8b5910fd538ccc5489e8edd3fde32917fad
```

You need rustup (the toolchain in `rust-toolchain.toml`) and CMake, plus:

- **Linux:**
  ```sh
  sudo apt-get install build-essential cmake pkg-config libx11-dev libx11-xcb-dev libxcb1-dev \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libfontconfig1-dev \
    libfreetype6-dev libvulkan-dev libssl-dev libclang-dev libasound2-dev
  ```
- **macOS:** Xcode Command Line Tools and `brew install cmake`. Metal shaders compile at launch, so `xcrun metal` isn't needed.
- **Windows:** Visual Studio 2022 Build Tools with **Desktop development with C++** and the Windows SDK. Run from a Developer PowerShell.

Run an offline preview, which never starts pi:

```sh
cargo run --locked -p pi-desktop -- --demo
```

Run for real with the backend from source (Node.js 22.19+):

```sh
npm ci --prefix packages/pi-desktop-backend
PI_DESKTOP_RPC_ENTRY="$PWD/packages/pi-desktop-backend/src/cli.mjs" \
  cargo run --release --locked -p pi-desktop -- --project /path/to/project
```

Real sessions use your provider credentials and can edit files, just like pi.

### Which backend runs

A session runs the first of these that is set:

1. `PI_DESKTOP_RPC_ENTRY`: a JavaScript entry, run with Node.js (`PI_DESKTOP_NODE`). Or `PI_DESKTOP_PI`: an executable.
2. Settings → PI DESKTOP → General → Backend.
3. The backend built into release builds (the `bundled-backend` feature).
4. `pi` from PATH.

`scripts/pi-rpc.mjs` runs a sibling `../pi` source checkout instead, when used as `PI_DESKTOP_RPC_ENTRY`.

## Package

```sh
bash scripts/package-linux.sh      # or package-macos.sh; ./scripts/package-windows.ps1 on Windows
```

Packaging needs Node.js 22.19+, [Bun](https://bun.sh) 1.4.2 and Python 3.11+. It compiles the backend into one executable, embeds it, and writes an archive to `dist/`. On macOS, `PI_DESKTOP_CODESIGN_IDENTITY` signs the backend for the hardened runtime. Details: [Built-in backend](docs/architecture.md#built-in-backend).

CI ([`.github/workflows/desktop.yml`](.github/workflows/desktop.yml)):
- **Every push:** runs the tests and lints on Linux and builds all four packages.
- **A `v*` tag:** also publishes them with `SHA256SUMS`, once everything passes.

## Test

```sh
cargo fmt -p pi_core -p pi_editor -p pi_jj -p pi_lsp_bridge -p pi_settings -p pi_terminal -p pi-desktop -- --check
cargo clippy --locked --workspace --all-targets --no-deps -- -D warnings
cargo test --locked --workspace
cargo test --locked -p pi_lsp_bridge --features fake-lsp
npm test --prefix packages/pi-desktop-backend
python3 -m unittest discover -s scripts -p 'test_packaging.py'
```

No test calls a model provider. `scripts/capture-*.sh` check the real window under Xvfb; see [docs/validation.md](docs/validation.md).

## Keys

| Keys | Action |
|---|---|
| Ctrl/Cmd+N | New session |
| Ctrl/Cmd+O | Open folder |
| Ctrl/Cmd+K | Search sessions |
| Ctrl/Cmd+B | Show or hide the sidebar |
| Ctrl+Shift+I | Show or hide the inspector |
| Ctrl+\` or Ctrl/Cmd+J | Terminal |
| Ctrl+Shift+T | Theme |
| Ctrl+Shift+D | Session diagnostics |
| Enter | Send, or steer while pi works |
| Alt/Option+Enter | Queue a follow-up |
| Escape | Stop |
| ⌥ held, in a file tab | Show which turn wrote each line |

## License

GPL-3.0-or-later ([LICENSE](LICENSE)), because the app links Zed's GPL crates. Fonts, icons, pi, Bun and the bundled npm packages are listed in [THIRD_PARTY.md](THIRD_PARTY.md) and `licenses/`.
