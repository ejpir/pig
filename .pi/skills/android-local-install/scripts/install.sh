#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: install.sh [--tag vX.Y.Z] [--serial ADB_SERIAL] [--helper-only|--android-only] [--keep]

Rebuilds and activates the full host helper, then safely installs the normal
dev.pi.android APK with bundled Local mode. The published APK is preferred when
its certificate matches; otherwise it is rebuilt with the matching configured
local keystore.
EOF
}

TAG=""
SERIAL="${PI_ANDROID_SERIAL:-}"
KEEP=0
DO_HELPER=1
DO_ANDROID=1
while (($#)); do
  case "$1" in
    --tag)
      TAG="${2:?--tag requires a value}"
      shift 2
      ;;
    --serial)
      SERIAL="${2:?--serial requires a value}"
      shift 2
      ;;
    --helper-only)
      DO_ANDROID=0
      shift
      ;;
    --android-only)
      DO_HELPER=0
      shift
      ;;
    --keep)
      KEEP=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"
read -r VERSION VERSION_CODE < <(python - <<'PY'
import tomllib
version = tomllib.load(open('Cargo.toml', 'rb'))['workspace']['package']['version']
major, minor, patch = (int(part) for part in version.split('.')[:3])
print(version, major * 1_000_000 + minor * 1_000 + patch)
PY
)
TAG=${TAG:-v$VERSION}
BUN_VERSION=$(python - <<'PY'
import json
print(json.load(open('crates/pi_local/package.json'))['engines']['bun'])
PY
)
SDK_VERSION=$(python - <<'PY'
import json
print(json.load(open('backend/durable/package.json'))['dependencies']['@earendil-works/pi-durable'])
PY
)

if ((DO_HELPER)); then
  HELPER_ARGS=()
  ((KEEP)) && HELPER_ARGS+=(--keep)
  "$ROOT/.pi/skills/android-local-install/scripts/rebuild-helper.sh" "${HELPER_ARGS[@]}"
fi
if ((!DO_ANDROID)); then
  exit 0
fi

ADB=${ADB:-"$HOME/Android/Sdk/platform-tools/adb"}
ANDROID_HOME=${ANDROID_HOME:-"$HOME/Android/Sdk"}
GH=${GH:-gh}
REPO=${PI_GITHUB_REPO:-$($GH repo view --json nameWithOwner --jq .nameWithOwner)}
APKSIGNER=${APKSIGNER:-$(find "$ANDROID_HOME/build-tools" -type f -name apksigner | sort -V | tail -1)}
if [[ ! -x "$ADB" || ! -x "$APKSIGNER" ]]; then
  echo "ADB or apksigner is unavailable under $ANDROID_HOME" >&2
  exit 1
fi

WORK=$(mktemp -d "${TMPDIR:-/tmp}/pi-android-install.XXXXXX")
cleanup() {
  if ((KEEP)); then
    echo "Kept diagnostic build directory: $WORK"
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

cert_digest() {
  "$APKSIGNER" verify --print-certs "$1" \
    | sed -n 's/^.*certificate SHA-256 digest: //p' \
    | head -1 \
    | tr '[:upper:]' '[:lower:]'
}

keystore_digest() {
  local output
  if [[ -n "${ANDROID_KEYSTORE:-}" ]]; then
    : "${ANDROID_KEYSTORE_PASSWORD:?ANDROID_KEYSTORE_PASSWORD is required with ANDROID_KEYSTORE}"
    local args=(-list -v -keystore "$ANDROID_KEYSTORE" -storepass:env ANDROID_KEYSTORE_PASSWORD)
    if [[ -n "${ANDROID_KEY_ALIAS:-}" ]]; then
      args+=(-alias "$ANDROID_KEY_ALIAS")
    fi
    output=$(keytool "${args[@]}")
  else
    local keystore="$HOME/.android/debug.keystore"
    [[ -f "$keystore" ]] || {
      echo "Installed app exists but $keystore is missing" >&2
      return 1
    }
    output=$(keytool -list -v -keystore "$keystore" -storepass android -alias androiddebugkey)
  fi
  sed -n 's/^.*SHA256: //p' <<<"$output" \
    | head -1 \
    | tr -d ':' \
    | tr '[:upper:]' '[:lower:]'
}

validate_local_apk() {
  python - "$1" "$VERSION" "$BUN_VERSION" "$SDK_VERSION" <<'PY'
import json, sys, zipfile
apk, version, bun, sdk = sys.argv[1:]
with zipfile.ZipFile(apk) as archive:
    names = set(archive.namelist())
    required = {
        'assets/local-runtime.json',
        'assets/bun-local.mjs',
        'lib/arm64-v8a/libbunembeddingpoc.so',
    }
    missing = required - names
    if missing:
        raise SystemExit(f'APK is not the normal bundled-Local build; missing {sorted(missing)}')
    metadata = json.loads(archive.read('assets/local-runtime.json'))
    expected = {'version': version, 'bun': bun, 'sdk': sdk, 'abi': 'arm64-v8a'}
    for key, value in expected.items():
        if metadata.get(key) != value:
            raise SystemExit(f'Local metadata mismatch for {key}: {metadata.get(key)!r} != {value!r}')
print(f"Local metadata verified: version={version} bun={bun} sdk={sdk}")
PY
}

"$ADB" start-server >/dev/null
if [[ -z "$SERIAL" ]]; then
  mapfile -t PHYSICAL < <(
    "$ADB" devices | awk -F '\t' 'NR > 1 && $2 == "device" && $1 !~ /^emulator-/ {print $1}'
  )
  if ((${#PHYSICAL[@]} == 0)); then
    echo "No connected physical Android device" >&2
    exit 1
  fi
  BASE=()
  for device in "${PHYSICAL[@]}"; do
    [[ "$device" == *" ("* ]] || BASE+=("$device")
  done
  if ((${#BASE[@]} == 1)); then
    SERIAL=${BASE[0]}
  elif ((${#PHYSICAL[@]} == 1)); then
    SERIAL=${PHYSICAL[0]}
  else
    printf 'Multiple physical devices are connected; pass --serial:\n' >&2
    printf '  %s\n' "${PHYSICAL[@]}" >&2
    exit 1
  fi
fi
[[ $("$ADB" -s "$SERIAL" get-state) == device ]]
echo "Selected physical device: $SERIAL"

CURRENT_REMOTE=$("$ADB" -s "$SERIAL" shell pm path dev.pi.android \
  | sed -n 's/^package://p' | tr -d '\r' | head -1)
if [[ -z "$CURRENT_REMOTE" ]]; then
  echo "dev.pi.android is not currently installed; refusing an unverified first install" >&2
  exit 1
fi
"$ADB" -s "$SERIAL" pull "$CURRENT_REMOTE" "$WORK/current.apk" >/dev/null
INSTALLED_CERT=$(cert_digest "$WORK/current.apk")
[[ -n "$INSTALLED_CERT" ]]
echo "Installed certificate SHA-256: $INSTALLED_CERT"

mkdir "$WORK/release"
"$GH" release download "$TAG" --repo "$REPO" \
  --pattern pi-android-arm64.apk --pattern SHA256SUMS --dir "$WORK/release"
grep '  pi-android-arm64.apk$' "$WORK/release/SHA256SUMS" \
  | (cd "$WORK/release" && sha256sum -c -)
RELEASE_APK="$WORK/release/pi-android-arm64.apk"
validate_local_apk "$RELEASE_APK"
RELEASE_CERT=$(cert_digest "$RELEASE_APK")
[[ -n "$RELEASE_CERT" ]]

APK="$RELEASE_APK"
SOURCE="published release APK"
if [[ "$RELEASE_CERT" != "$INSTALLED_CERT" ]]; then
  echo "Published APK signer differs; checking the configured local keystore"
  LOCAL_CERT=$(keystore_digest)
  if [[ "$LOCAL_CERT" != "$INSTALLED_CERT" ]]; then
    echo "Configured local keystore does not match the installed app; refusing to uninstall or replace data" >&2
    exit 1
  fi

  "$GH" release download "$TAG" --repo "$REPO" \
    --pattern pi-bun-android-arm64-runtime.tar.gz --dir "$WORK/release"
  grep '  pi-bun-android-arm64-runtime.tar.gz$' "$WORK/release/SHA256SUMS" \
    | (cd "$WORK/release" && sha256sum -c -)
  mkdir "$WORK/runtime"
  tar -xzf "$WORK/release/pi-bun-android-arm64-runtime.tar.gz" -C "$WORK/runtime"
  BUN_LIBRARY="$WORK/runtime/libbunembeddingpoc.so"
  [[ -f "$BUN_LIBRARY" && -f "$WORK/runtime/LICENSE.bun.md" && -f "$WORK/runtime/BUN-LINKED-NOTICES.txt" ]]

  npm install --prefix "$WORK/bun-host" --ignore-scripts --no-save "bun@$BUN_VERSION" >/dev/null
  (cd "$WORK/bun-host/node_modules/bun" && node install.js)
  HOST_BUN="$WORK/bun-host/node_modules/.bin/bun"
  [[ $($HOST_BUN --version) == "$BUN_VERSION" ]]

  npm ci --ignore-scripts --prefix crates/pi_local >/dev/null
  npm ci --ignore-scripts --omit=dev --prefix backend/durable >/dev/null

  NDK_ROOT=""
  for candidate in "$ANDROID_HOME/ndk/29.0.14206865" "$ANDROID_HOME/ndk"/*; do
    [[ -f "$candidate/source.properties" ]] || continue
    if grep -q '^Pkg.Revision = 29\.0\.14206865$' "$candidate/source.properties"; then
      NDK_ROOT="$candidate"
      break
    fi
  done
  [[ -n "$NDK_ROOT" ]] || {
    echo "Android NDK 29.0.14206865 is not installed" >&2
    exit 1
  }

  APK="$WORK/pi-android-arm64.apk"
  PATH="$(dirname "$HOST_BUN"):$PATH" \
  ANDROID_HOME="$ANDROID_HOME" \
  ANDROID_NDK_ROOT="$NDK_ROOT" \
  CARGO_TARGET_DIR="$WORK/target" \
  PI_ANDROID_BUN_LIBRARY="$BUN_LIBRARY" \
    python crates/pi_android/scripts/build_apk.py --out "$APK"
  validate_local_apk "$APK"
  BUILT_CERT=$(cert_digest "$APK")
  if [[ "$BUILT_CERT" != "$INSTALLED_CERT" ]]; then
    echo "Built APK signer does not match installed app; refusing installation" >&2
    exit 1
  fi
  SOURCE="local normal APK rebuilt with matching keystore"
fi

FINAL_CERT=$(cert_digest "$APK")
[[ "$FINAL_CERT" == "$INSTALLED_CERT" ]]
echo "Certificate match verified; installing $SOURCE"
"$ADB" -s "$SERIAL" install -r --no-incremental "$APK"

PACKAGE_INFO=$("$ADB" -s "$SERIAL" shell dumpsys package dev.pi.android)
grep -Fq "versionCode=$VERSION_CODE" <<<"$PACKAGE_INFO"
grep -Fq "versionName=$VERSION" <<<"$PACKAGE_INFO"
UPDATED_REMOTE=$("$ADB" -s "$SERIAL" shell pm path dev.pi.android \
  | sed -n 's/^package://p' | tr -d '\r' | head -1)
"$ADB" -s "$SERIAL" pull "$UPDATED_REMOTE" "$WORK/installed.apk" >/dev/null
[[ $(cert_digest "$WORK/installed.apk") == "$INSTALLED_CERT" ]]
validate_local_apk "$WORK/installed.apk"

echo "Installed dev.pi.android $VERSION in place from $SOURCE"
echo "App data was preserved; no uninstall or package clear was performed"
