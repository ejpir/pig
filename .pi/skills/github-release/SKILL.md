---
name: github-release
description: Prepare, validate, push, tag, and verify a Pi GitHub release without bypassing CI. Use a clean release worktree, wait for green branch CI before tagging, and let the tag workflow publish the artifacts and notes.
compatibility: Linux host with Git, GitHub CLI, Python 3, Rust/Cargo, Bun, Node/npm, and access to ejpir/pig.
---

# GitHub Release

Use this skill for a production `vMAJOR.MINOR.PATCH` release after the intended changes have been reviewed.

## Safety invariants

- Release from the intended `main` commit; never force-push.
- Keep unrelated worktree changes out of the release commit. Stage explicit paths, not `git add -A`.
- Bump every version location: root workspace, `pi_local`, both lockfiles, and both macOS plist fields.
- Commit `docs/releases/vX.Y.Z.md`; CI uses it verbatim as the GitHub release body.
- Run local checks before committing.
- Push the release commit and wait for the **Desktop** branch workflow to finish successfully before creating a tag.
- Create one annotated tag and push only that tag.
- Do not run `gh release create` locally. Tag CI assembles and validates the complete artifact matrix, writes `SHA256SUMS`, and publishes the release.
- Never delete or move an existing release tag to recover from a failure. Inspect the failed run and release state first.
- Prefer a clean temporary worktree for the remote release step, especially when the development checkout contains unrelated changes.

## Version preparation

For `vX.Y.Z`, update:

- `Cargo.toml` workspace package version.
- `Cargo.lock` workspace package versions.
- `crates/pi_local/Cargo.toml`.
- `crates/pi_local/Cargo.lock`.
- `packaging/macos/Info.plist`:
  - `CFBundleShortVersionString` = `X.Y.Z`
  - monotonically increasing `CFBundleVersion`.
- `docs/releases/vX.Y.Z.md`.

Validate the version boundary:

```bash
python3 scripts/release_desktop.py --tag vX.Y.Z
cargo metadata --locked --no-deps --format-version 1 >/dev/null
cargo metadata --locked --no-deps --format-version 1 \
  --manifest-path crates/pi_local/Cargo.toml >/dev/null
```

## Release notes

Use concise, user-visible Markdown. A normal structure is:

```markdown
## Highlights

**One bold summary paragraph.**

### Feature area

- User-visible behavior.
- Compatibility or migration details.

### Runtime, validation and safety

- Packaging or platform notes.
- Experimental limitations and safe upgrade instructions.

**Full changelog:** https://github.com/ejpir/pig/compare/PREVIOUS...CURRENT
```

Do not promise unfinished design studies or uncommitted behavior.

## Local validation

Match `.github/workflows/desktop.yml`. At minimum, run all formatting, tests and warnings-denied Clippy commands affected by the release. For a full release candidate, run the complete checks job documented in `docs/validation.md` and the workflow, including Durable/Bun checks, `pi_local` generated-bundle freshness, Android feature matrices, packaging tests, and native screen validation.

Always finish with:

```bash
git diff --check
git diff --cached --check
```

## Commit

Stage only reviewed release files:

```bash
git add -- <explicit paths>
git diff --cached --stat
git diff --cached
git commit -m "Prepare vX.Y.Z"
```

Do not include scratch HTML, local captures, credentials, downloaded artifacts, build directories, or unrelated design experiments.

## Push, tag and publish

Run the included script from a clean checkout or clean temporary worktree at the committed release SHA:

```bash
.pi/skills/github-release/scripts/release.sh \
  --tag vX.Y.Z \
  --repo ejpir/pig \
  --branch main
```

The script:

1. Verifies a clean checkout, matching versions and release notes.
2. Refuses existing local/remote tags or GitHub releases.
3. Fast-forwards `origin/main` to the current SHA.
4. Waits for the branch **Desktop** workflow to succeed.
5. Rechecks that remote `main` still points to the tested SHA.
6. Creates and pushes one annotated tag.
7. Waits for tag CI, including the tag-only publish job.
8. Verifies the non-draft GitHub release and lists its assets.

If branch CI fails, fix it in a new commit and rerun from a clean worktree. If tag CI fails, do not retag or manually pre-create the release; inspect the failed run and any partially published assets first.

## Required completion report

Report:

- release version, commit SHA and annotated tag
- local validation performed
- branch workflow URL/result
- tag workflow URL/result
- GitHub release URL and asset count
- whether signing/publishing was performed by CI
- confirmation that no force-push, manual release creation, or unrelated staging occurred
