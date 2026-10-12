#!/usr/bin/env bash
# Nick's Pi Desktop launchpad. No installs or builds unless you ask for them.
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd -- "$ROOT"

cyan='' pink='' bold='' reset=''
if [[ -t 1 && -z ${NO_COLOR+x} && ${TERM:-dumb} != dumb ]]; then
  cyan=$'\033[36m'
  pink=$'\033[35m'
  bold=$'\033[1m'
  reset=$'\033[0m'
fi

banner() {
  printf '%s' "$pink"
  cat <<'PIG'
     ^-----^
    ( o   o )    N I C K ' S   L A U N C H P A D
     (  @  )     Small pig. Big ideas.
PIG
  printf '%s\n' "$reset"
}

usage() {
  printf '%sUsage:%s ./nick.sh [command] [extra arguments]\n\n' "$bold" "$reset"
  cat <<'HELP'
  status   Show the project dashboard (default; read-only)
  demo     Launch the offline preview; never starts pi
  build    Build pi-desktop in debug mode
  check    Type-check pi-desktop
  test     Run workspace tests
  help     Show this help

Examples:
  ./nick.sh demo
  ./nick.sh build --release
  ./nick.sh test -p pi_core
  ./nick.sh demo --light

Build/check/test arguments go to Cargo; demo arguments go to the app.
Run from any directory. Set NO_COLOR=1 to disable colors.
HELP
}

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'Missing required tool: %s\n' "$1" >&2
    exit 127
  fi
}

run() {
  require "$1"
  printf '\n%s>>>%s' "$cyan" "$reset"
  printf ' %q' "$@"
  printf '\n\n'
  exec "$@"
}

status() {
  banner
  printf '%sProject%s  %s\n\n' "$bold" "$reset" "$ROOT"
  printf '%sGit%s\n' "$cyan" "$reset"
  if command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
    git -c color.ui=false status --short --branch
  else
    printf '  Git metadata unavailable.\n'
  fi

  printf '\n%sToolbox%s\n' "$cyan" "$reset"
  local tool
  for tool in cargo rustup cmake pkg-config python3; do
    if command -v "$tool" >/dev/null 2>&1; then
      printf '  [ok] %s\n' "$tool"
    else
      printf '  [--] %s (not on PATH)\n' "$tool"
    fi
  done

  printf '\n%sZed checkout%s\n' "$cyan" "$reset"
  if [[ -f "$ROOT/../zed/crates/gpui/Cargo.toml" ]]; then
    printf '  [ok] ../zed\n'
  else
    printf '  [--] Expected ../zed; see README.md for build setup.\n'
  fi
  printf '\n'
  usage
}

command_name=${1:-status}
if (( $# > 0 )); then
  shift
fi

case "$command_name" in
  status|help|-h|--help)
    if (( $# > 0 )); then
      printf '%s does not accept extra arguments.\n' "$command_name" >&2
      exit 2
    fi
    if [[ "$command_name" == status ]]; then status; else usage; fi
    ;;
  demo)
    run cargo run --locked -p pi-desktop -- --demo "$@"
    ;;
  build)
    run cargo build --locked -p pi-desktop "$@"
    ;;
  check)
    run cargo check --locked -p pi-desktop "$@"
    ;;
  test)
    run cargo test --locked --workspace "$@"
    ;;
  *)
    printf 'Unknown command: %s. Try ./nick.sh help\n' "$command_name" >&2
    exit 2
    ;;
esac
