use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Result;

use crate::{
    ChangeId, Env, FileStatus, LineKind, ObjectId as _, OperationKind, Project, UndoConflict, Vcs,
    detect, short_change_id,
};

fn env() -> Env {
    Env::isolated("Test User", "test@example.com")
}

fn write(root: &Path, path: &str, text: &str) {
    fs::write(root.join(path), text).unwrap();
}

fn read(root: &Path, path: &str) -> Option<String> {
    fs::read_to_string(root.join(path)).ok()
}

/// Runs a command and returns its trimmed stdout, failing the test on error.
fn run(mut command: Command, args: &[&str]) -> String {
    let output = command.args(args).output().expect("command runs");
    assert!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// Runs git with no user or system config, so a developer's settings cannot
/// change the result.
fn git(root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args([
            "-c",
            "user.name=Git User",
            "-c",
            "user.email=git@example.com",
        ])
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ]);
    run(command, args)
}

#[test]
fn a_turn_is_its_own_change() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    write(root, "a.txt", "one\n");
    let mut project = Project::init_with(root, &env())?;

    let turn = project.begin_turn("Change a")?;
    write(root, "a.txt", "two\n");
    write(root, "b.txt", "new\n");
    let change = project.end_turn(&turn)?.expect("the turn changed files");

    // Only the turn's edits: a.txt existed before the turn started.
    let changes = project.changes(&change)?;
    let summary: Vec<_> = changes
        .iter()
        .map(|c| (c.path.as_str(), c.status, c.added, c.removed))
        .collect();
    assert_eq!(
        summary,
        [
            ("a.txt", FileStatus::Modified, 1, 1),
            ("b.txt", FileStatus::Added, 1, 0)
        ]
    );
    assert_eq!(
        changes[0].hunks[0].lines,
        [
            (LineKind::Removed, "one".to_owned()),
            (LineKind::Added, "two".to_owned())
        ]
    );

    // Later edits go to a new empty change on top, not into the turn.
    let turn_commit = project.visible_commit(&change)?.unwrap();
    assert_eq!(turn_commit.description(), "Change a");
    let wc = project.wc_commit()?.unwrap();
    assert_eq!(wc.parent_ids(), [turn_commit.id().clone()]);

    // The next turn with edits takes over that empty change instead of
    // stacking another under it.
    let next = project.begin_turn("Change b")?;
    write(root, "c.txt", "more\n");
    assert_eq!(project.end_turn(&next)?.as_ref(), Some(wc.change_id()));
    Ok(())
}

#[test]
fn a_turn_without_edits_writes_nothing() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    write(root, "a.txt", "one\n");
    let mut project = Project::init_with(root, &env())?;
    let before = project.operation_id().clone();
    let turn = project.begin_turn("Only read files")?;
    assert_eq!(project.end_turn(&turn)?, None);
    assert_eq!(project.operation_id(), &before);
    Ok(())
}

#[test]
fn edits_made_before_a_turn_stay_out_of_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut project = Project::init_with(root, &env())?;
    write(root, "mine.txt", "by hand\n");
    let turn = project.begin_turn("Add a")?;
    write(root, "a.txt", "one\n");
    let change = project.end_turn(&turn)?.unwrap();

    let paths: Vec<_> = project
        .changes(&change)?
        .into_iter()
        .map(|c| c.path)
        .collect();
    assert_eq!(paths, ["a.txt"]);
    let turn_commit = project.visible_commit(&change)?.unwrap();
    let below = &turn_commit.parent_ids()[0];
    assert_ne!(below, project.wc_commit()?.unwrap().id());
    Ok(())
}

#[test]
fn undo_restores_files_and_redo_brings_them_back() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    write(root, "a.txt", "one\n");
    let mut project = Project::init_with(root, &env())?;
    let turn = project.begin_turn("Change a")?;
    write(root, "a.txt", "two\n");
    write(root, "b.txt", "new\n");
    let change = project.end_turn(&turn)?.unwrap();

    let undo = project.undo_turn(&change)?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("one\n"));
    assert_eq!(read(root, "b.txt"), None);
    assert!(project.visible_commit(&change)?.is_none());

    project.redo(&undo)?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
    assert_eq!(read(root, "b.txt").as_deref(), Some("new\n"));
    assert!(project.visible_commit(&change)?.is_some());
    Ok(())
}

#[test]
fn a_recorded_turn_is_found_again_after_undo_and_reopening() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let (change, commit, undo) = {
        let mut project = Project::init_with(root, &env())?;
        let turn = project.begin_turn("Add a\n")?;
        write(root, "a.txt", "one\n");
        let change = project.end_turn(&turn)?.unwrap();
        let commit = project.visible_commit(&change)?.unwrap().id().clone();
        (change.clone(), commit, project.undo_turn(&change)?)
    };
    let mut project = Project::open_with(root, &env())?;
    let undone = project.recorded_turn(&change, &commit)?;
    assert!(!undone.visible, "an undone turn is hidden");
    assert_eq!(undone.description, "Add a");
    assert_eq!(
        undone.files.len(),
        1,
        "its files still read from the recorded commit"
    );

    project.redo(&undo)?;
    let redone = project.recorded_turn(&change, &commit)?;
    assert!(redone.visible && redone.commit == commit);
    assert_eq!(ChangeId::try_from_hex(change.hex()), Some(change));
    Ok(())
}

#[test]
fn redo_is_refused_after_new_edits() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut project = Project::init_with(root, &env())?;
    let turn = project.begin_turn("Add a")?;
    write(root, "a.txt", "one\n");
    let change = project.end_turn(&turn)?.unwrap();
    let undo = project.undo_turn(&change)?;

    write(root, "c.txt", "mine\n");
    assert!(project.redo(&undo).is_err());
    assert_eq!(read(root, "c.txt").as_deref(), Some("mine\n"));
    assert_eq!(read(root, "a.txt"), None);
    Ok(())
}

#[test]
fn git_follows_turns_in_a_colocated_repo() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "a.txt", "one\n");
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "Initial"]);

    let mut project = Project::init_with(root, &env())?;
    assert!(project.is_colocated());
    let turn = project.begin_turn("Change a")?;
    write(root, "a.txt", "two\n");
    project.end_turn(&turn)?;

    // Git HEAD is the working copy's parent: the turn, on top of git's commit.
    assert_eq!(git(root, &["log", "--format=%s"]), "Change a\nInitial");
    assert_eq!(git(root, &["status", "--porcelain"]), "");

    // A commit made with git becomes the base for the next turn.
    write(root, "a.txt", "three\n");
    git(root, &["commit", "-q", "-am", "By hand"]);
    let next = project.begin_turn("Change again")?;
    write(root, "a.txt", "four\n");
    project.end_turn(&next)?;
    assert_eq!(
        git(root, &["log", "--format=%s"]),
        "Change again\nBy hand\nChange a\nInitial"
    );
    Ok(())
}

#[test]
fn a_turn_is_not_split_when_git_moves_the_working_copy() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    git(root, &["init", "-q"]);
    write(root, "a.txt", "one\n");
    git(root, &["add", "a.txt"]);
    git(root, &["commit", "-q", "-m", "Initial"]);
    let mut project = Project::init_with(root, &env())?;

    let turn = project.begin_turn("Change a")?;
    write(root, "a.txt", "two\n");
    git(root, &["commit", "-q", "-am", "By hand during the run"]);
    assert!(project.end_turn(&turn).is_err());
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
    Ok(())
}

#[test]
fn reopening_keeps_turns() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let change = {
        let mut project = Project::init_with(root, &env())?;
        let turn = project.begin_turn("Add a")?;
        write(root, "a.txt", "one\n");
        project.end_turn(&turn)?.unwrap()
    };
    let project = Project::open_with(root, &env())?;
    assert_eq!(project.changes(&change)?.len(), 1);
    Ok(())
}

#[test]
fn detect_finds_the_nearest_workspace() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let nested = root.join("src/deep");
    fs::create_dir_all(&nested)?;
    assert_eq!(detect(&nested), Vcs::None);
    git(root, &["init", "-q"]);
    assert_eq!(
        detect(&nested),
        Vcs::Git {
            root: root.to_owned()
        }
    );
    Project::init_with(root, &env())?;
    assert_eq!(
        detect(&nested),
        Vcs::Jj {
            root: root.to_owned()
        }
    );
    Ok(())
}

/// The jj CLI from `PI_JJ_CLI`, for tests that check pi_jj and the CLI can share
/// a repo. Point it at the version jj-lib is pinned to (and again when upgrading
/// jj-lib); those tests are skipped otherwise.
fn jj_cli(root: &Path) -> Option<impl Fn(&[&str]) -> String + '_> {
    let cli = std::env::var_os("PI_JJ_CLI")?;
    Some(move |args: &[&str]| {
        let mut command = Command::new(&cli);
        command
            .current_dir(root)
            .env("JJ_CONFIG", "")
            .env("JJ_USER", "Jj User")
            .env("JJ_EMAIL", "jj@example.com");
        run(command, args)
    })
}

#[test]
fn the_jj_cli_and_pi_jj_share_a_repo() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let Some(jj) = jj_cli(root) else {
        return Ok(());
    };
    jj(&["git", "init", "--colocate"]);
    write(root, "a.txt", "one\n");
    jj(&["commit", "-m", "By jj"]);

    let mut project = Project::open_with(root, &env())?;
    let turn = project.begin_turn("Change a")?;
    write(root, "a.txt", "two\n");
    let change = project.end_turn(&turn)?.unwrap();

    let log = [
        "log",
        "--no-graph",
        "-r",
        "::@- ~ root()",
        "-T",
        "description.first_line() ++ \"\\n\"",
    ];
    assert_eq!(jj(&log), "Change a\nBy jj");
    assert_eq!(
        jj(&["diff", "--summary", "-r", &short_change_id(&change)]),
        "M a.txt"
    );
    assert!(jj(&["status"]).contains("The working copy has no changes."));

    // `jj undo` reverts recording the turn: the edits are back in the working
    // copy, undescribed (the turn had taken over that empty change), and pi_jj
    // follows.
    jj(&["undo"]);
    let mut project = Project::open_with(root, &env())?;
    project.snapshot()?;
    let wc = project.wc_commit()?.unwrap();
    assert_eq!((wc.change_id(), wc.description()), (&change, ""));
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
    Ok(())
}

#[test]
fn project_can_move_to_a_background_thread() {
    fn assert_send<T: Send + 'static>() {}
    assert_send::<Project>();
}

#[test]
fn an_earlier_turn_undoes_alone_unless_later_turns_build_on_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut project = Project::init_with(root, &env())?;
    let turn = |project: &mut Project, name: &str, text: &str| -> Result<ChangeId> {
        let turn = project.begin_turn(name)?;
        write(root, name, text);
        Ok(project.end_turn(&turn)?.unwrap())
    };
    let first = turn(&mut project, "a.txt", "one\n")?;
    let second = turn(&mut project, "a.txt", "two\n")?;
    let independent = turn(&mut project, "b.txt", "other\n")?;

    // The second turn edits the file the first created: undoing the first
    // alone would leave conflict markers, so nothing happens.
    let before = project.operation_id().clone();
    let error = project.undo_turn(&first).unwrap_err();
    let conflict = error.downcast_ref::<UndoConflict>().expect("a conflict");
    assert_eq!(conflict.later, std::slice::from_ref(&second));
    assert!(!conflict.working_copy);
    assert_eq!(project.operation_id(), &before);
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));

    // A turn nothing depends on undoes alone; then the chain undoes newest first.
    project.undo_turn(&independent)?;
    assert_eq!(read(root, "b.txt"), None);
    project.undo_turn(&second)?;
    project.undo_turn(&first)?;
    assert_eq!(read(root, "a.txt"), None);
    Ok(())
}

#[test]
fn two_projects_on_one_repo_see_each_others_turns() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut first = Project::init_with(root, &env())?;
    let mut second = Project::open_with(root, &env())?;

    let turn = first.begin_turn("Add a")?;
    write(root, "a.txt", "one\n");
    let a = first.end_turn(&turn)?.unwrap();

    // `second` loaded before that turn; its next turn builds on it.
    let turn = second.begin_turn("Add b")?;
    write(root, "b.txt", "two\n");
    let b = second.end_turn(&turn)?.unwrap();
    let a_commit = second.visible_commit(&a)?.unwrap();
    assert_eq!(
        second.visible_commit(&b)?.unwrap().parent_ids(),
        [a_commit.id().clone()]
    );
    let paths: Vec<_> = second.changes(&b)?.into_iter().map(|c| c.path).collect();
    assert_eq!(paths, ["b.txt"]);
    Ok(())
}

#[test]
fn an_open_project_sees_jj_cli_operations() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let Some(jj) = jj_cli(root) else {
        return Ok(());
    };
    let mut project = Project::init_with(root, &env())?;
    let turn = project.begin_turn("Add a")?;
    write(root, "a.txt", "one\n");
    let a = project.end_turn(&turn)?.unwrap();

    // An operation that leaves the working copy alone, while pi_jj stays open.
    jj(&[
        "describe",
        "--ignore-working-copy",
        "-r",
        &short_change_id(&a),
        "-m",
        "Renamed",
    ]);
    let turn = project.begin_turn("Add b")?;
    write(root, "b.txt", "two\n");
    project.end_turn(&turn)?.unwrap();

    let log = [
        "log",
        "--no-graph",
        "-r",
        "all()",
        "-T",
        "if(divergent, \"divergent \") ++ description.first_line() ++ \"\\n\"",
    ];
    let log = jj(&log);
    assert!(!log.contains("divergent"), "{log}");
    assert!(log.contains("Renamed"), "{log}");
    Ok(())
}

/// Records a turn that writes `path`, as the app does around a run.
fn turn_writing(project: &mut Project, root: &Path, path: &str, text: &str) -> Result<ChangeId> {
    let turn = project.begin_turn(&format!("Write {path}"))?;
    write(root, path, text);
    Ok(project.end_turn(&turn)?.unwrap())
}

#[test]
fn one_file_of_a_turn_restores_unless_a_later_turn_edits_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    write(root, "a.txt", "one\n");
    let mut project = Project::init_with(root, &env())?;
    let turn = project.begin_turn("Change a and b")?;
    write(root, "a.txt", "two\n");
    write(root, "b.txt", "new\n");
    let change = project.end_turn(&turn)?.unwrap();
    let later = turn_writing(&mut project, root, "b.txt", "newer\n")?;

    // b.txt: a later turn builds on it, so nothing happens.
    let before = project.operation_id().clone();
    let error = project.restore_file(&change, "b.txt").unwrap_err();
    let conflict = error.downcast_ref::<UndoConflict>().expect("a conflict");
    assert_eq!(conflict.later, std::slice::from_ref(&later));
    assert_eq!(project.operation_id(), &before);

    // a.txt goes back; the turn keeps its other file.
    project.restore_file(&change, "a.txt")?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("one\n"));
    assert_eq!(read(root, "b.txt").as_deref(), Some("newer\n"));
    let files: Vec<_> = project
        .changes(&change)?
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(files, ["b.txt"]);
    Ok(())
}

#[test]
fn a_conflicting_undo_can_take_later_turns_along_or_keep_the_conflict() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut project = Project::init_with(root, &env())?;
    let first = turn_writing(&mut project, root, "a.txt", "one\n")?;
    let second = turn_writing(&mut project, root, "a.txt", "two\n")?;

    // Both in one operation, which one redo brings back.
    let undo = project.undo_turns(&[second.clone(), first.clone()])?;
    assert_eq!(read(root, "a.txt"), None);
    project.redo(&undo)?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));

    // Or the later turn keeps jj's conflict and the file gets markers.
    let kept = project.undo_turn_keeping_conflicts(&first)?;
    assert_eq!(kept.files, ["a.txt"]);
    assert!(read(root, "a.txt").unwrap().contains("<<<<<<<"));
    assert!(project.visible_commit(&second)?.unwrap().has_conflict());
    project.redo(&kept.operation)?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
    Ok(())
}

#[test]
fn the_operation_log_lists_and_restores_every_step() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut project = Project::init_with(root, &env())?;
    turn_writing(&mut project, root, "a.txt", "one\n")?;
    write(root, "mine.txt", "by hand\n");
    let second = turn_writing(&mut project, root, "a.txt", "two\n")?;

    let operations = project.operations(50)?;
    let kinds: Vec<_> = operations
        .iter()
        .map(|op| (op.kind, op.description.as_str()))
        .collect();
    assert_eq!(
        kinds[..3],
        [
            (
                OperationKind::Pi,
                format!("pi: record turn {}", short_change_id(&second)).as_str()
            ),
            (OperationKind::Snapshot, "snapshot working copy"),
            (OperationKind::Snapshot, "snapshot working copy"),
        ]
    );
    assert!(
        operations
            .windows(2)
            .all(|pair| pair[0].time >= pair[1].time)
    );

    // Back to the snapshot that recorded the hand edit, before the second turn
    // wrote: a.txt as it was then, the hand edit kept.
    let target = &operations[2];
    let files: Vec<_> = project
        .operation_files(&target.id)?
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(files, ["a.txt"]);
    let restore = project.restore_operation(&target.id)?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("one\n"));
    assert_eq!(read(root, "mine.txt").as_deref(), Some("by hand\n"));

    // Restoring is an operation too.
    let before_restore = project.operations(50)?[1].id.clone();
    assert_ne!(before_restore, restore);
    project.restore_operation(&before_restore)?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("two\n"));
    Ok(())
}

#[test]
fn what_one_command_changed_goes_back_to_before_it() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    write(root, "a.txt", "one\n");
    write(root, "b.txt", "keep\n");
    let mut project = Project::init_with(root, &env())?;
    let turn = project.begin_turn("Clean up")?;
    write(root, "c.txt", "pi's edit\n");
    let before = project.take_snapshot()?;
    // The command deletes a file and changes another.
    fs::remove_file(root.join("a.txt"))?;
    write(root, "b.txt", "changed\n");
    let (_, files) = project.changed_since(&before)?;
    let paths: Vec<_> = files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["a.txt", "b.txt"]);

    project.restore_paths(&before, &["a.txt".into(), "b.txt".into()])?;
    assert_eq!(read(root, "a.txt").as_deref(), Some("one\n"));
    assert_eq!(read(root, "b.txt").as_deref(), Some("keep\n"));
    assert_eq!(
        read(root, "c.txt").as_deref(),
        Some("pi's edit\n"),
        "not the command's"
    );
    // The turn still records the files as they end up.
    let change = project.end_turn(&turn)?.unwrap();
    let paths: Vec<_> = project
        .changes(&change)?
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(paths, ["c.txt"]);
    Ok(())
}

#[test]
fn the_jj_tools_answer_in_text() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut project = Project::init_with(root, &env())?;
    let first = turn_writing(&mut project, root, "a.txt", "one\ntwo\n")?;
    let second = turn_writing(&mut project, root, "a.txt", "one\nthree\n")?;

    let log = project.log_text(10)?;
    let lines: Vec<_> = log.lines().collect();
    assert!(lines[0].contains("(working copy)"), "{log}");
    assert!(lines[1].starts_with(&short_change_id(&second)), "{log}");
    assert!(
        lines[1].ends_with("Write a.txt") && lines[1].contains("+1 -1"),
        "{log}"
    );
    assert!(lines[2].starts_with(&short_change_id(&first)), "{log}");
    assert!(
        project
            .log_text(1)?
            .ends_with("(older changes not shown)\n")
    );

    let diff = project.diff_text(&short_change_id(&second), None)?;
    assert_eq!(
        diff,
        "--- a/a.txt\n+++ b/a.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n"
    );
    assert_eq!(
        project.diff_text(&short_change_id(&second), Some("b.txt"))?,
        "The change does not touch that file.\n"
    );
    let show = project.show_text(&short_change_id(&first))?;
    assert!(
        show.contains("Write a.txt") && show.contains("A a.txt  +2 -0"),
        "{show}"
    );
    assert!(project.show_text("nosuchchange").is_err());
    Ok(())
}

#[test]
fn lines_know_the_turn_that_wrote_them() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    write(root, "a.txt", "mine\n");
    let mut project = Project::init_with(root, &env())?;
    let first = turn_writing(&mut project, root, "a.txt", "mine\nfirst\n")?;
    let second = turn_writing(&mut project, root, "a.txt", "mine\nfirst\nsecond\n")?;
    // An unsaved line in the editor has no turn yet.
    let turns = project.annotate("a.txt", "mine\nfirst\nsecond\nunsaved\n")?;
    assert_eq!(turns.lines, [None, Some(first.clone()), Some(second), None]);
    assert_eq!(turns.descriptions[&first], "Write a.txt");
    Ok(())
}

#[test]
fn a_workspace_works_apart_and_its_turns_come_back() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("main");
    fs::create_dir(&root)?;
    write(&root, "a.txt", "one\n");
    let mut main = Project::init_with(&root, &env())?;
    turn_writing(&mut main, &root, "a.txt", "two\n")?;

    let side = dir.path().join("side");
    let mut workspace = main.add_workspace(&side, None)?;
    assert_eq!(workspace.workspace_name_text(), "side");
    assert_eq!(
        read(&side, "a.txt").as_deref(),
        Some("two\n"),
        "files as of the last turn"
    );
    let theirs = turn_writing(&mut workspace, &side, "b.txt", "from the side\n")?;
    // Meanwhile the main folder moves on, and has an edit of its own.
    turn_writing(&mut main, &root, "c.txt", "main\n")?;
    write(&root, "mine.txt", "by hand\n");
    assert_eq!(read(&root, "b.txt"), None);

    main.bring_in("side")?;
    assert_eq!(read(&root, "b.txt").as_deref(), Some("from the side\n"));
    assert_eq!(read(&root, "c.txt").as_deref(), Some("main\n"));
    assert_eq!(read(&root, "mine.txt").as_deref(), Some("by hand\n"));
    assert!(main.visible_commit(&theirs)?.is_some());
    assert!(main.bring_in("side").is_err(), "nothing left to bring in");

    // A workspace with the files as they were before a turn, for a fork.
    let first = turn_writing(&mut main, &root, "a.txt", "three\n")?;
    let before = main.add_workspace(&dir.path().join("fork"), Some(&first))?;
    assert_eq!(read(before.root(), "a.txt").as_deref(), Some("two\n"));
    Ok(())
}
