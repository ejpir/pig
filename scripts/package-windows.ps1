$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$target = if ($env:PI_DESKTOP_TARGET) { $env:PI_DESKTOP_TARGET } else { 'x86_64-pc-windows-msvc' }
if ($target -ne 'x86_64-pc-windows-msvc') { throw "Unsupported Windows release target: $target" }
# The built-in pi, pi's own release binary (notices need npm): see docs/architecture.md.
$piOut = Join-Path (Get-Location) 'artifacts/pi'
python scripts/fetch_pi.py --platform windows-x64 --out $piOut --notices
if ($LASTEXITCODE -ne 0) { throw 'Fetching pi failed' }
$env:PI_DESKTOP_BACKEND_ARCHIVE = Join-Path $piOut 'pi-windows-x64.tar.gz'
cargo build --locked --release -p pi-desktop -p pi_remote --target $target --features pi-desktop/bundled-backend,pi_remote/bundled-backend
if ($LASTEXITCODE -ne 0) { throw 'Windows build failed' }
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
python scripts/package_desktop.py --target $target --binary "$targetDir/$target/release/pi-desktop.exe" --notices (Join-Path $piOut 'pi-notices.txt')
if ($LASTEXITCODE -ne 0) { throw 'Windows packaging failed' }
python scripts/package_remote.py --target $target --binary "$targetDir/$target/release/pi-desktop-remote.exe"
if ($LASTEXITCODE -ne 0) { throw 'Windows helper packaging failed' }
Write-Output 'Packaged Windows amd64 (unsigned portable ZIP).'
