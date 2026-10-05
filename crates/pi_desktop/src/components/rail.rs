//! The run rail (design/visual-workflow A, E): each step of a run is a soft tile
//! in the gutter, with one hue per kind of step. A live turn also shows the
//! stages still ahead of it as dashed tiles, which its calls fill in.
//! Presentation only; nothing here reads or changes session data.
use super::*;

/// The gutter left of assistant content that holds the spine and the nodes.
pub const RAIL: Pixels = px(36.);
const NODE: Pixels = px(22.);

/// What a step did, from the tools it called. Unknown tools are `Other`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StepKind {
    Explore,
    Search,
    Change,
    /// A command that runs tests, linters or type checks.
    Check,
    Run,
    Other,
    #[default]
    Reasoning,
}

impl StepKind {
    pub fn of_tool(name: &str) -> Self {
        match name {
            "read" | "ls" | "find" => Self::Explore,
            "grep" => Self::Search,
            "edit" | "write" => Self::Change,
            "bash" => Self::Run,
            _ => Self::Other,
        }
    }
    /// Like `of_tool`, but a command that runs checks is a check, and so is one
    /// that runs a file the agent `written` earlier: trying a script it wrote.
    pub fn of_call(name: &str, args: &serde_json::Value, written: &[String]) -> Self {
        match Self::of_tool(name) {
            Self::Run
                if args["command"]
                    .as_str()
                    .is_some_and(|command| is_check(command) || runs_written(command, written)) =>
            {
                Self::Check
            }
            kind => kind,
        }
    }
    /// A step that changed files reads as a change even if it also looked around;
    /// running outranks looking, and looking outranks bare reasoning.
    pub fn merge(self, other: Self) -> Self {
        if other.rank() < self.rank() {
            other
        } else {
            self
        }
    }
    /// Steps break where the phase of work changes: looking around (reads and
    /// searches together), changing files, running commands, other tools.
    pub fn phase(self) -> Self {
        match self {
            Self::Search => Self::Explore,
            kind => kind,
        }
    }
    fn rank(self) -> u8 {
        match self {
            Self::Change => 0,
            Self::Check => 1,
            Self::Run => 2,
            Self::Other => 3,
            Self::Explore => 4,
            Self::Search => 5,
            Self::Reasoning => 6,
        }
    }
    pub fn hue(self, theme: Theme) -> Hsla {
        match self {
            Self::Explore => theme.steel,
            Self::Search => theme.violet,
            Self::Change => theme.orange,
            Self::Check => theme.green,
            // The study's legend: reading and running share steel.
            Self::Run => theme.steel,
            Self::Other => theme.secondary,
            Self::Reasoning => theme.muted,
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Explore => "eye",
            Self::Search => "magnifying_glass",
            Self::Change => "pencil",
            Self::Check => "shield",
            Self::Run => "terminal",
            Self::Other => "box",
            Self::Reasoning => "sparkle",
        }
    }
}

/// Whether a command runs checks: a word like `test`, `lint` or `check` in it.
pub fn is_check(command: &str) -> bool {
    const CHECKS: &[&str] = &[
        "check",
        "test",
        "tests",
        "lint",
        "clippy",
        "tsc",
        "typecheck",
        "pytest",
        "vitest",
        "jest",
        "eslint",
        "mypy",
        "ruff",
        "build",
        "shellcheck",
        "bats",
        "vet",
        "rspec",
        "phpunit",
        "unittest",
        "tox",
        "ctest",
    ];
    command
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| CHECKS.contains(&word))
}

/// Whether a command runs a file in `written`, directly or through an
/// interpreter: `./nick.sh`, `bash nick.sh`, `python3 tool.py --help`.
fn runs_written(command: &str, written: &[String]) -> bool {
    const INTERPRETERS: &[&str] = &[
        "bash", "sh", "zsh", "fish", "dash", "python", "python3", "node", "deno", "bun", "ruby",
        "perl", "php", "lua", "tsx", "ts-node",
    ];
    let is_written = |word: &str| {
        let word = word.trim_matches(['"', '\'']).trim_start_matches("./");
        !word.is_empty()
            && written.iter().any(|path| {
                let path = path.trim_start_matches("./");
                path == word
                    || path.ends_with(&format!("/{word}"))
                    || word.ends_with(&format!("/{path}"))
            })
    };
    command.split(['&', '|', ';', '\n']).any(|segment| {
        let mut words = segment
            .split_whitespace()
            .skip_while(|word| word.contains('=') || matches!(*word, "sudo" | "time" | "env"));
        match words.next() {
            Some(program) if is_written(program) => true,
            Some(program) if INTERPRETERS.contains(&program) => words
                .find(|word| !word.starts_with('-'))
                .is_some_and(is_written),
            _ => false,
        }
    })
}

/// Whether a command only looks around: every part of it lists, prints or
/// searches, and nothing is redirected into a file.
fn only_looks(command: &str) -> bool {
    const LOOKS: [&str; 14] = [
        "ls", "cat", "head", "tail", "rg", "grep", "find", "fd", "wc", "pwd", "tree", "stat",
        "file", "which",
    ];
    const GIT: [&str; 5] = ["status", "diff", "log", "show", "blame"];
    !command.contains('>')
        && command
            .split(['&', '|', ';', '\n'])
            .map(str::split_whitespace)
            .all(|mut words| match words.next() {
                None => true,
                Some("git") => words.next().is_some_and(|sub| GIT.contains(&sub)),
                Some(program) => LOOKS.contains(&program),
            })
}

/// The template a coding turn follows: Understand → Change → Verify → Hand off.
/// A live turn shows the stages after the furthest one its calls reached, so
/// the whole flow is visible before work arrives. It describes the shape of a
/// turn, never a promise that a stage will run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    Understand,
    Change,
    Verify,
    HandOff,
}

impl Stage {
    pub const ALL: [Self; 4] = [Self::Understand, Self::Change, Self::Verify, Self::HandOff];

    /// The stage a call of `kind` belongs to. Unknown tools say nothing about it.
    pub fn of(kind: StepKind, args: &serde_json::Value) -> Option<Self> {
        match kind {
            StepKind::Explore | StepKind::Search => Some(Self::Understand),
            StepKind::Change => Some(Self::Change),
            StepKind::Check => Some(Self::Verify),
            StepKind::Run if args["command"].as_str().is_some_and(only_looks) => {
                Some(Self::Understand)
            }
            StepKind::Run => Some(Self::Change),
            StepKind::Other | StepKind::Reasoning => None,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Understand => "Understand",
            Self::Change => "Change",
            Self::Verify => "Verify",
            Self::HandOff => "Hand off",
        }
    }
    /// What the stage is for, shown while it is still ahead.
    pub fn purpose(self) -> &'static str {
        match self {
            Self::Understand => "Read and search the code",
            Self::Change => "Edit files",
            Self::Verify => "Run checks",
            Self::HandOff => "Summary",
        }
    }
    pub fn slug(self) -> &'static str {
        match self {
            Self::Understand => "understand",
            Self::Change => "change",
            Self::Verify => "verify",
            Self::HandOff => "handoff",
        }
    }
    pub fn hue(self, theme: Theme) -> Hsla {
        match self {
            Self::Understand => StepKind::Explore.hue(theme),
            Self::Change => StepKind::Change.hue(theme),
            Self::Verify => StepKind::Check.hue(theme),
            Self::HandOff => theme.accent,
        }
    }
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Understand => StepKind::Explore.glyph(),
            Self::Change => StepKind::Change.glyph(),
            Self::Verify => StepKind::Check.glyph(),
            Self::HandOff => "send",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// Every call reported a result.
    Done,
    /// The run is still in this step.
    Live,
    Failed,
    /// A settled step with calls that never reported a result.
    Incomplete,
    /// A queued message: the rail's dashed future.
    Queued,
    /// A stage of the template the turn has not reached.
    Planned,
}

/// A step's mark on the rail: a soft tile tinted in its hue. Live tiles breathe;
/// nothing else moves.
pub fn rail_node(
    id: impl Into<ElementId>,
    kind: StepKind,
    state: NodeState,
    theme: Theme,
) -> AnyElement {
    tile(id, kind.glyph(), kind.hue(theme), state, theme)
}

/// A template stage's tile, in the same language as the steps that fill it.
pub fn stage_node(
    id: impl Into<ElementId>,
    stage: Stage,
    state: NodeState,
    theme: Theme,
) -> AnyElement {
    tile(id, stage.glyph(), stage.hue(theme), state, theme)
}

fn tile(
    id: impl Into<ElementId>,
    glyph: &'static str,
    hue: Hsla,
    state: NodeState,
    theme: Theme,
) -> AnyElement {
    let (glyph, fill, border, glyph_color) = match state {
        NodeState::Done => (glyph, theme.tint(hue), theme.tint(hue), hue),
        NodeState::Live => (glyph, theme.composer, hue.opacity(0.7), hue),
        NodeState::Failed => (
            "warning",
            theme.tint(theme.coral),
            theme.tint(theme.coral),
            theme.coral,
        ),
        NodeState::Incomplete => (glyph, theme.canvas, theme.line_strong, theme.muted),
        NodeState::Queued => ("queue", theme.canvas, theme.line_strong, theme.muted),
        NodeState::Planned => (glyph, theme.canvas, theme.line_strong, theme.muted),
    };
    let node = div()
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size(NODE)
        .rounded(px(6.))
        .bg(fill)
        .border_1()
        .border_color(border)
        .when(
            matches!(
                state,
                NodeState::Incomplete | NodeState::Queued | NodeState::Planned
            ),
            |node| node.border_dashed(),
        )
        .child(icon(glyph, glyph_color).size(px(12.)));
    if state != NodeState::Live {
        return node.into_any_element();
    }
    node.with_animation(
        id,
        Animation::new(Duration::from_millis(1800))
            .repeat()
            .with_max_fps(30.)
            .with_easing(pulsating_between(0., 1.)),
        move |node, t| node.shadow(halo(hue, t)),
    )
    .into_any_element()
}

/// The study's live halo. GPUI keeps a shadow's corner radius as it spreads, so
/// the glow comes from blur, which stays round. `t` in 0..=1 widens and fades it.
pub fn halo(hue: Hsla, t: f32) -> Vec<BoxShadow> {
    vec![BoxShadow::new(px(0.), px(0.), hue.opacity(0.3 - 0.16 * t)).blur_radius(px(4. + 5. * t))]
}

/// The status dot beside the composer: a hue, breathing while `live`.
pub fn status_dot(id: impl Into<ElementId>, hue: Hsla, live: bool) -> AnyElement {
    let dot = div().flex_shrink_0().size(px(8.)).rounded_full().bg(hue);
    if !live {
        return dot.into_any_element();
    }
    dot.with_animation(
        id,
        Animation::new(Duration::from_millis(1800))
            .repeat()
            .with_max_fps(30.)
            .with_easing(pulsating_between(0., 1.)),
        move |dot, t| dot.shadow(halo(hue, t)),
    )
    .into_any_element()
}

/// How far a tile sits from the gutter's edge, inside a live step's card.
pub const INSET: Pixels = px(5.);

/// Places a node on the rail beside content that starts `RAIL` to its right.
pub fn on_rail(node: AnyElement, top: Pixels) -> Div {
    div().absolute().left(-RAIL + INSET).top(top).child(node)
}

/// Five blocks scaled to the change, as a review queue shows them.
pub fn diff_blocks(added: usize, removed: usize, theme: Theme) -> Div {
    let total = added + removed;
    let filled = total.min(5);
    let green = if total == 0 {
        0
    } else {
        ((filled * added) as f32 / total as f32).round() as usize
    }
    .max(usize::from(added > 0))
    .min(filled);
    let red = filled - green;
    h_flex()
        .flex_shrink_0()
        .gap(px(2.))
        .children((0..5).map(move |i| {
            div().size(px(7.)).rounded(px(1.5)).bg(if i < green {
                theme.green
            } else if i < green + red {
                theme.coral
            } else {
                theme.line
            })
        }))
}

/// The card shadow from the study: a hairline lift, not a floating panel.
pub fn lift(theme: Theme) -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(
            px(0.),
            px(1.),
            gpui::black().opacity(if theme.light { 0.05 } else { 0.25 }),
        )
        .blur_radius(px(2.)),
    ]
}

#[cfg(test)]
mod tests {
    // Not `super::*`: gpui's `test` attribute would shadow the standard one.
    use super::{Stage, StepKind, is_check};
    use serde_json::json;

    #[test]
    fn a_step_is_named_by_its_most_consequential_call() {
        let kind = |names: &[&str]| {
            names.iter().fold(StepKind::Reasoning, |kind, name| {
                kind.merge(StepKind::of_tool(name))
            })
        };
        assert_eq!(kind(&[]), StepKind::Reasoning);
        assert_eq!(kind(&["grep"]), StepKind::Search);
        assert_eq!(kind(&["grep", "read"]), StepKind::Explore);
        assert_eq!(kind(&["read", "bash"]), StepKind::Run);
        assert_eq!(kind(&["read", "edit", "bash"]), StepKind::Change);
        assert_eq!(kind(&["mcp_lookup", "read"]), StepKind::Other);
    }

    #[test]
    fn calls_land_in_the_template_stage_they_serve() {
        let stage = |name: &str, command: &str| {
            let args = json!({"command": command});
            Stage::of(StepKind::of_call(name, &args, &[]), &args)
        };
        assert_eq!(stage("grep", ""), Some(Stage::Understand));
        assert_eq!(stage("write", ""), Some(Stage::Change));
        assert_eq!(stage("bash", "npm run check"), Some(Stage::Verify));
        assert_eq!(
            stage("bash", "cargo test -p pi-desktop"),
            Some(Stage::Verify)
        );
        assert_eq!(stage("bash", "vision-check"), Some(Stage::Verify));
        assert_eq!(
            stage("bash", "git status && rg signature | head"),
            Some(Stage::Understand)
        );
        assert_eq!(
            stage("bash", "chmod +x nick.sh && ls -l nick.sh"),
            Some(Stage::Change)
        );
        assert_eq!(stage("bash", "cat a > b"), Some(Stage::Change));
        assert_eq!(stage("mcp_lookup", ""), None);
        assert!(!is_check("cat latest.log"), "words, not substrings");
    }

    #[test]
    fn running_a_script_the_agent_wrote_verifies_it() {
        let written = [
            "/home/me/proj/nick.sh".to_owned(),
            "tools/report.py".to_owned(),
        ];
        let kind =
            |command: &str| StepKind::of_call("bash", &json!({"command": command}), &written);
        assert_eq!(kind("./nick.sh"), StepKind::Check);
        assert_eq!(
            kind("chmod +x nick.sh && ./nick.sh --dry-run"),
            StepKind::Check
        );
        assert_eq!(kind("bash -x \"nick.sh\""), StepKind::Check);
        assert_eq!(kind("DEBUG=1 python3 tools/report.py"), StepKind::Check);
        assert_eq!(kind("shellcheck deploy.sh"), StepKind::Check);
        assert_eq!(
            kind("./deploy.sh"),
            StepKind::Run,
            "a script the agent did not write"
        );
        assert_eq!(
            kind("chmod +x nick.sh"),
            StepKind::Run,
            "naming it is not running it"
        );
        assert_eq!(kind("bash -c 'echo hi'"), StepKind::Run);
    }
}
