$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$target = if ($env:PI_DESKTOP_TARGET) { $env:PI_DESKTOP_TARGET } else { 'x86_64-pc-windows-msvc' }
if ($target -ne 'x86_64-pc-windows-msvc') { throw "Unsupported Windows release target: $target" }
# The built-in backend (needs Node.js 22.19+ and Bun): see docs/architecture.md.
$backendOut = Join-Path (Get-Location) 'artifacts/backend'
npm ci --prefix packages/pi-desktop-backend
if ($LASTEXITCODE -ne 0) { throw 'Backend install failed' }
node packages/pi-desktop-backend/scripts/build-binary.mjs --platform windows-x64 --out $backendOut
if ($LASTEXITCODE -ne 0) { throw 'Backend build failed' }
$env:PI_DESKTOP_BACKEND_ARCHIVE = Join-Path $backendOut 'pi-desktop-backend-windows-x64.tar.zst'
cargo build --locked --release -p pi-desktop --target $target --features bundled-backend
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
python scripts/package_desktop.py --target $target --binary "$targetDir/$target/release/pi-desktop.exe" --notices (Join-Path $backendOut 'pi-desktop-backend-notices.txt')
if ($LASTEXITCODE -ne 0) { throw 'Windows packaging failed' }
Write-Output 'Packaged Windows amd64 (unsigned portable ZIP).'
