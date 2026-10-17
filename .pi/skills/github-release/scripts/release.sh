#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: release.sh --tag vX.Y.Z [--repo owner/repo] [--branch main] [--remote origin]

Push the current clean release commit, wait for green branch CI, create and push
one annotated tag, wait for tag CI to publish, and verify the GitHub release.
EOF
}

TAG=
REPO=ejpir/pig
BRANCH=main
REMOTE=origin
while (($#)); do
  case "$1" in
    --tag) TAG=${2:?missing value for --tag}; shift 2 ;;
    --repo) REPO=${2:?missing value for --repo}; shift 2 ;;
    --branch) BRANCH=${2:?missing value for --branch}; shift 2 ;;
    --remote) REMOTE=${2:?missing value for --remote}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ ! ${TAG:-} =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][A-Za-z0-9]+([.-][A-Za-z0-9]+)*)?$ ]]; then
  echo "--tag must look like v0.1.0 or v0.1.0-rc.1" >&2
  exit 2
fi

for command in git gh python3 cargo; do
  command -v "$command" >/dev/null || { echo "Missing command: $command" >&2; exit 1; }
done

ROOT=$(git rev-parse --show-toplevel)
cd "$ROOT"

if [[ -n $(git status --porcelain) ]]; then
  echo "Release checkout is not clean. Use a clean temporary worktree." >&2
  git status --short >&2
  exit 1
fi

SHA=$(git rev-parse HEAD)
VERSION=${TAG#v}
NOTES="docs/releases/$TAG.md"
[[ -s "$NOTES" ]] || { echo "Missing nonempty release notes: $NOTES" >&2; exit 1; }

python3 scripts/release_desktop.py --tag "$TAG"
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cargo metadata --locked --no-deps --format-version 1 \
  --manifest-path crates/pi_local/Cargo.toml >/dev/null
python3 - "$VERSION" <<'PY'
import pathlib, sys, tomllib
expected = sys.argv[1]
root = tomllib.loads(pathlib.Path("Cargo.toml").read_text())["workspace"]["package"]["version"]
local = tomllib.loads(pathlib.Path("crates/pi_local/Cargo.toml").read_text())["package"]["version"]
plist = pathlib.Path("packaging/macos/Info.plist").read_text()
if root != expected or local != expected:
    raise SystemExit(f"Version mismatch: workspace={root}, pi_local={local}, expected={expected}")
if f"<key>CFBundleShortVersionString</key><string>{expected}</string>" not in plist:
    raise SystemExit("macOS short version does not match")
PY

gh auth status --hostname github.com >/dev/null
git fetch --quiet "$REMOTE" "+refs/heads/$BRANCH:refs/remotes/$REMOTE/$BRANCH" --tags

if git show-ref --verify --quiet "refs/tags/$TAG"; then
  echo "Local tag already exists: $TAG" >&2
  exit 1
fi
if [[ -n $(git ls-remote "$REMOTE" "refs/tags/$TAG") ]]; then
  echo "Remote tag already exists: $TAG" >&2
  exit 1
fi
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
  echo "GitHub release already exists: $TAG" >&2
  exit 1
fi

REMOTE_SHA=$(git rev-parse "refs/remotes/$REMOTE/$BRANCH")
if ! git merge-base --is-ancestor "$REMOTE_SHA" "$SHA"; then
  echo "$REMOTE/$BRANCH is not an ancestor of the release commit; refusing a non-fast-forward push." >&2
  exit 1
fi

echo "Pushing release commit $SHA to $REMOTE/$BRANCH"
git push "$REMOTE" "$SHA:refs/heads/$BRANCH"

remote_branch_sha() {
  git ls-remote "$REMOTE" "refs/heads/$BRANCH" | awk 'NR == 1 {print $1}'
}

if [[ $(remote_branch_sha) != "$SHA" ]]; then
  echo "Remote branch does not point to the release commit after push." >&2
  exit 1
fi

find_run() {
  local head=$1
  gh run list --repo "$REPO" --workflow Desktop --event push --commit "$SHA" --limit 20 \
    --json databaseId,headBranch,headSha \
    --jq "[.[] | select(.headSha == \"$SHA\" and .headBranch == \"$head\")] | .[0].databaseId // empty"
}

LAST_RUN_ID=
wait_for_run() {
  local head=$1 run_id=
  echo "Waiting for Desktop workflow for $head at $SHA"
  for _ in $(seq 1 120); do
    run_id=$(find_run "$head")
    [[ -n "$run_id" ]] && break
    sleep 5
  done
  [[ -n "$run_id" ]] || { echo "No Desktop workflow appeared for $head" >&2; return 1; }
  echo "Workflow: https://github.com/$REPO/actions/runs/$run_id"
  gh run watch "$run_id" --repo "$REPO" --compact --exit-status
  LAST_RUN_ID=$run_id
}

wait_for_run "$BRANCH"
BRANCH_RUN=$LAST_RUN_ID
if [[ $(remote_branch_sha) != "$SHA" ]]; then
  echo "Remote $BRANCH moved after CI; refusing to tag an unconfirmed branch head." >&2
  exit 1
fi
if [[ $(git rev-parse HEAD) != "$SHA" ]]; then
  echo "Local HEAD moved during release." >&2
  exit 1
fi

git tag -a "$TAG" "$SHA" -m "$TAG"
git push "$REMOTE" "refs/tags/$TAG"
wait_for_run "$TAG"
TAG_RUN=$LAST_RUN_ID

for _ in $(seq 1 60); do
  if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    break
  fi
  sleep 5
done

gh release view "$TAG" --repo "$REPO" \
  --json tagName,name,isDraft,isPrerelease,publishedAt,url,assets \
  --jq '{tagName,name,isDraft,isPrerelease,publishedAt,url,assetCount:(.assets|length),assets:[.assets[].name]}'

echo "Released $TAG from $SHA"
echo "Branch workflow: https://github.com/$REPO/actions/runs/$BRANCH_RUN"
echo "Tag workflow: https://github.com/$REPO/actions/runs/$TAG_RUN"
