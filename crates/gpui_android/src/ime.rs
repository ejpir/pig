//! The on-screen keyboard's view of the text, kept in step with GPUI.
//!
//! The keyboard (IME) edits a mirror of the text around the selection, held in
//! Java. After each batch of edits Java reports the mirror's whole state, and
//! the difference becomes calls on GPUI's input handler. The other way, when the
//! app changes its text or selection, the app's state is pushed to the mirror.
//!
//! Both sides edit at once, so every report carries a sequence number and every
//! push names the report it was based on. Java drops a push that is based on a
//! stale report; the keyboard's newer report then arrives and the app's state
//! is pushed again. A report that acknowledges a push is not applied to GPUI,
//! which already holds that state.
//!
//! All offsets are UTF-16 code units, as in both Android's and GPUI's APIs.

use gpui::{Autocapitalize, TextInputAction, TextInputConfiguration};
use std::{collections::VecDeque, ops::Range};

/// How much text either side of the selection the keyboard sees.
const CONTEXT: usize = 2000;
/// How close the selection may come to the edge of the mirrored text before
/// the mirror is moved, unless the edge is the end of the editable text.
const MARGIN: usize = 200;

/// The mirror's text, selection and composing region (offsets into `text`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ImeState {
    pub text: String,
    pub selection: Range<usize>,
    pub composing: Option<Range<usize>>,
}

/// A call on GPUI's input handler, in document offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Edit {
    Replace {
        range: Range<usize>,
        text: String,
    },
    /// Replaces `range` with `text`, marks it as composing, and selects
    /// `selection`, which is relative to the start of `text`.
    Compose {
        range: Range<usize>,
        text: String,
        selection: Range<usize>,
    },
    /// Ends composing, keeping the text.
    Commit,
    Select(Range<usize>),
    /// The keyboard's return key typed a line break at the caret. It is sent
    /// as an enter key press, so the app's own enter bindings run, as with a
    /// hardware keyboard.
    Enter,
}

/// The app's text input around its selection, in document offsets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppText {
    /// The text of the document range starting at `start`.
    pub text: String,
    pub start: usize,
    pub selection: Range<usize>,
    pub marked: Option<Range<usize>>,
}

/// A state to send to the keyboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Push {
    /// The report this push is based on; Java drops it if the keyboard has moved on.
    pub basis: i32,
    pub id: i32,
    pub state: ImeState,
    /// Whether the keyboard must start over with this text. Not for a change a
    /// key caused: the keyboard sent the key and expects it, as with Android's
    /// own text fields, and starting over would cancel a key it is holding,
    /// such as delete.
    pub restart: bool,
}

#[derive(Default)]
pub(crate) struct Mirror {
    /// What the keyboard's editor holds, as last reported.
    state: ImeState,
    /// The document offset of the mirrored text.
    start: usize,
    seq: i32,
    next_push: i32,
    /// Pushes not yet acknowledged: id, document offset, state.
    pending: VecDeque<(i32, usize, ImeState)>,
}

impl Mirror {
    /// Forgets the mirrored text, for a new or unfocused input.
    pub fn reset(&mut self) {
        *self = Self {
            seq: self.seq,
            next_push: self.next_push,
            ..Self::default()
        };
    }

    /// The keyboard reported its state. Returns the edits that bring the
    /// document in line; none when the report acknowledges push `push_id`.
    pub fn reported(&mut self, seq: i32, push_id: i32, state: ImeState) -> Vec<Edit> {
        self.seq = seq;
        if push_id != 0 {
            while let Some((id, start, _)) = self.pending.pop_front() {
                if id == push_id {
                    self.start = start;
                    break;
                }
            }
            self.state = state;
            return Vec::new();
        }
        let edits = diff(&self.state, &state, self.start);
        self.state = state;
        edits
    }

    /// The document range to mirror for a selection: the current one while the
    /// selection stays well inside it, else a new one around the selection.
    pub fn window(&self, selection: &Range<usize>, editable: &Range<usize>) -> Range<usize> {
        let current = self.start..self.start + len16(&self.state.text);
        let room_before = selection.start.saturating_sub(current.start);
        let room_after = current.end.saturating_sub(selection.end);
        let inside = current.start <= selection.start && selection.end <= current.end;
        if !self.state.text.is_empty()
            && inside
            && (room_before >= MARGIN || current.start <= editable.start)
            && (room_after >= MARGIN || current.end >= editable.end)
        {
            return current;
        }
        let start = selection.start.saturating_sub(CONTEXT).max(editable.start);
        let end = (selection.end + CONTEXT).min(editable.end).max(start);
        start..end
    }

    /// The app's current state, `keyed` when a key caused the change. Returns
    /// a push when the keyboard's mirror differs and the same state is not
    /// already on its way.
    pub fn sync(&mut self, app: AppText, keyed: bool) -> Option<Push> {
        let len = len16(&app.text);
        let relative = |range: &Range<usize>| {
            range.start.saturating_sub(app.start).min(len)
                ..range.end.saturating_sub(app.start).min(len)
        };
        let state = ImeState {
            selection: relative(&app.selection),
            composing: app
                .marked
                .as_ref()
                .filter(|marked| app.start <= marked.start && marked.end <= app.start + len)
                .map(relative),
            text: app.text,
        };
        if app.start == self.start && state == self.state {
            return None;
        }
        if self
            .pending
            .back()
            .is_some_and(|(_, start, pushed)| *start == app.start && *pushed == state)
        {
            return None;
        }
        self.next_push += 1;
        self.pending
            .push_back((self.next_push, app.start, state.clone()));
        Some(Push {
            basis: self.seq,
            id: self.next_push,
            state,
            restart: !keyed,
        })
    }
}

/// The edits that turn the document's `old` mirror into `new`.
fn diff(old: &ImeState, new: &ImeState, start: usize) -> Vec<Edit> {
    let shift = |range: &Range<usize>| range.start + start..range.end + start;
    let a: Vec<u16> = old.text.encode_utf16().collect();
    let b: Vec<u16> = new.text.encode_utf16().collect();
    // Marks `region` of the new text as composing, replacing `replaced` of the document.
    let compose_over = |replaced: &Range<usize>, region: &Range<usize>| Edit::Compose {
        range: shift(replaced),
        text: String::from_utf16_lossy(&b[region.clone()]),
        selection: new
            .selection
            .start
            .saturating_sub(region.start)
            .min(region.len())
            ..new
                .selection
                .end
                .saturating_sub(region.start)
                .min(region.len()),
    };
    let compose = |region: &Range<usize>| compose_over(region, region);
    let selection_in = |region: &Range<usize>| {
        region.start <= new.selection.start && new.selection.end <= region.end
    };
    let mut edits = Vec::new();
    if a == b {
        if new.composing != old.composing {
            match &new.composing {
                Some(region) => edits.push(compose(region)),
                None => edits.push(Edit::Commit),
            }
        }
        let selected = new
            .composing
            .as_ref()
            .is_some_and(|region| new.composing != old.composing && selection_in(region));
        if new.selection != old.selection && !selected {
            edits.push(Edit::Select(shift(&new.selection)));
        }
        return edits;
    }

    let (prefix, suffix) = common_affixes(&a, &b);
    let changed = prefix..b.len() - suffix;
    match &new.composing {
        // Typing within a composition: replace the old composing text whole.
        Some(region) if region.start <= changed.start && changed.end <= region.end => {
            let grown = b.len() as isize - a.len() as isize;
            let old_region = region.start..(region.end as isize - grown) as usize;
            edits.push(compose_over(&old_region, region));
            if !selection_in(region) {
                edits.push(Edit::Select(shift(&new.selection)));
            }
        }
        None if old.composing.is_none()
            && prefix == a.len() - suffix
            && b[changed.clone()] == [u16::from(b'\n')]
            && old.selection == (prefix..prefix)
            && new.selection == (changed.end..changed.end) =>
        {
            edits.push(Edit::Enter);
        }
        composing => {
            if old.composing.is_some() {
                edits.push(Edit::Commit);
            }
            edits.push(Edit::Replace {
                range: shift(&(prefix..a.len() - suffix)),
                text: String::from_utf16_lossy(&b[changed.clone()]),
            });
            match composing {
                Some(region) => {
                    edits.push(compose(region));
                    if !selection_in(region) {
                        edits.push(Edit::Select(shift(&new.selection)));
                    }
                }
                None if new.selection != (changed.end..changed.end) => {
                    edits.push(Edit::Select(shift(&new.selection)));
                }
                None => {}
            }
        }
    }
    edits
}

/// The lengths of the common prefix and suffix, never splitting a surrogate pair.
fn common_affixes(a: &[u16], b: &[u16]) -> (usize, usize) {
    let mut prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    if prefix > 0 && is_high_surrogate(a[prefix - 1]) && prefix < a.len().min(b.len()) {
        prefix -= 1;
    }
    let room = a.len().min(b.len()) - prefix;
    let mut suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    if suffix > 0 && is_low_surrogate(a[a.len() - suffix]) {
        suffix -= 1;
    }
    (prefix, suffix)
}

fn is_high_surrogate(unit: u16) -> bool {
    (0xD800..0xDC00).contains(&unit)
}

fn is_low_surrogate(unit: u16) -> bool {
    (0xDC00..0xE000).contains(&unit)
}

pub(crate) fn len16(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Android's `EditorInfo.inputType` and `imeOptions` for a text input.
///
/// Inputs that take line breaks are multi-line, so the keyboard shows a return
/// key; the others show the requested action.
pub(crate) fn editor_info(configuration: &TextInputConfiguration) -> (i32, i32) {
    const TEXT: i32 = 0x1;
    const CAP_CHARACTERS: i32 = 0x1000;
    const CAP_WORDS: i32 = 0x2000;
    const CAP_SENTENCES: i32 = 0x4000;
    const AUTO_CORRECT: i32 = 0x8000;
    const MULTI_LINE: i32 = 0x20000;
    const NO_SUGGESTIONS: i32 = 0x80000;

    let mut input_type = TEXT;
    input_type |= match configuration.autocapitalize {
        Autocapitalize::None => 0,
        Autocapitalize::Words => CAP_WORDS,
        Autocapitalize::Sentences => CAP_SENTENCES,
        Autocapitalize::Characters => CAP_CHARACTERS,
    };
    if configuration.autocorrect {
        input_type |= AUTO_CORRECT;
    }
    if !configuration.suggestions {
        input_type |= NO_SUGGESTIONS;
    }
    let action = match configuration.input_action {
        TextInputAction::Unspecified | TextInputAction::Enter => {
            input_type |= MULTI_LINE;
            if configuration.input_action == TextInputAction::Enter {
                1 // IME_ACTION_NONE
            } else {
                0 // IME_ACTION_UNSPECIFIED
            }
        }
        TextInputAction::Go => 2,
        TextInputAction::Search => 3,
        TextInputAction::Send => 4,
        TextInputAction::Next => 5,
        TextInputAction::Done => 6,
        TextInputAction::Previous => 7,
    };
    (input_type, action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(text: &str, selection: Range<usize>, composing: Option<Range<usize>>) -> ImeState {
        ImeState {
            text: text.into(),
            selection,
            composing,
        }
    }

    fn mirror(text: &str, caret: usize) -> Mirror {
        let mut mirror = Mirror::default();
        mirror.reported(1, 0, state(text, caret..caret, None));
        mirror
    }

    #[test]
    fn typing_within_a_composition_replaces_the_composing_word() {
        let mut mirror = mirror("hi ", 3);
        let edits = mirror.reported(2, 0, state("hi h", 4..4, Some(3..4)));
        assert_eq!(
            edits,
            [Edit::Compose {
                range: 3..3,
                text: "h".into(),
                selection: 1..1
            }]
        );
        let edits = mirror.reported(3, 0, state("hi he", 5..5, Some(3..5)));
        assert_eq!(
            edits,
            [Edit::Compose {
                range: 3..4,
                text: "he".into(),
                selection: 2..2
            }]
        );
    }

    #[test]
    fn autocorrect_commits_the_corrected_word() {
        let mut mirror = mirror("", 0);
        mirror.reported(2, 0, state("helo", 4..4, Some(0..4)));
        let edits = mirror.reported(3, 0, state("hello ", 6..6, None));
        assert_eq!(
            edits,
            [
                Edit::Commit,
                Edit::Replace {
                    range: 3..4,
                    text: "lo ".into()
                }
            ]
        );
    }

    #[test]
    fn backspace_into_a_word_recomposes_it() {
        let mut mirror = mirror("hello", 5);
        let edits = mirror.reported(2, 0, state("hell", 4..4, Some(0..4)));
        assert_eq!(
            edits,
            [Edit::Compose {
                range: 0..5,
                text: "hell".into(),
                selection: 4..4
            }]
        );
    }

    #[test]
    fn finishing_or_starting_a_composition_without_text_changes() {
        let mut mirror = mirror("", 0);
        mirror.reported(2, 0, state("word", 4..4, Some(0..4)));
        assert_eq!(
            mirror.reported(3, 0, state("word", 4..4, None)),
            [Edit::Commit]
        );
        assert_eq!(
            mirror.reported(4, 0, state("word", 4..4, Some(0..4))),
            [Edit::Compose {
                range: 0..4,
                text: "word".into(),
                selection: 4..4
            }]
        );
    }

    #[test]
    fn the_return_key_presses_enter_but_pasted_lines_are_text() {
        let mut typed = mirror("hi", 2);
        assert_eq!(
            typed.reported(2, 0, state("hi\n", 3..3, None)),
            [Edit::Enter]
        );
        let mut pasted = mirror("hi", 2);
        assert_eq!(
            pasted.reported(2, 0, state("hi\nyo", 5..5, None)),
            [Edit::Replace {
                range: 2..2,
                text: "\nyo".into()
            }]
        );
    }

    #[test]
    fn moving_the_cursor_selects_in_document_offsets() {
        let mut mirror = Mirror {
            start: 100,
            ..Mirror::default()
        };
        mirror.reported(1, 0, state("abc", 3..3, None));
        assert_eq!(
            mirror.reported(2, 0, state("abc", 1..1, None)),
            [Edit::Select(101..101)]
        );
    }

    #[test]
    fn emoji_are_replaced_whole() {
        let mut mirror = mirror("a😀b", 3);
        // 😀 is two UTF-16 units; replacing it with 😃 differs only in the low unit.
        let edits = mirror.reported(2, 0, state("a😃b", 3..3, None));
        assert_eq!(
            edits,
            [Edit::Replace {
                range: 1..3,
                text: "😃".into()
            }]
        );
    }

    #[test]
    fn app_changes_are_pushed_once_and_acknowledgements_are_not_reapplied() {
        let mut mirror = mirror("hi", 2);
        let app = AppText {
            text: "hi!".into(),
            start: 0,
            selection: 3..3,
            marked: None,
        };
        let push = mirror.sync(app.clone(), false).expect("the app added text");
        assert_eq!(push.basis, 1);
        assert_eq!(push.state, state("hi!", 3..3, None));
        assert!(push.restart, "the keyboard did not see it coming");
        assert_eq!(mirror.sync(app.clone(), false), None, "already on its way");
        assert!(
            mirror.reported(2, push.id, push.state.clone()).is_empty(),
            "the acknowledgement is the app's own change"
        );
        assert_eq!(mirror.sync(app, false), None, "now in step");
    }

    #[test]
    fn a_held_delete_key_keeps_the_keyboard_going() {
        let mut mirror = mirror("hello", 5);
        let app = AppText {
            text: "hell".into(),
            start: 0,
            selection: 4..4,
            marked: None,
        };
        let push = mirror
            .sync(app, true)
            .expect("the delete key removed a letter");
        assert!(
            !push.restart,
            "the keyboard sent the key, and may be repeating it"
        );
    }

    #[test]
    fn keyboards_offer_what_the_input_asks_for() {
        let code = TextInputConfiguration::default();
        assert_eq!(
            editor_info(&code),
            (0x1 | 0x20000 | 0x80000, 0),
            "plain multi-line text without suggestions"
        );
        let message = TextInputConfiguration {
            autocorrect: true,
            autocapitalize: Autocapitalize::Sentences,
            suggestions: true,
            input_action: TextInputAction::Send,
        };
        assert_eq!(editor_info(&message), (0x1 | 0x4000 | 0x8000, 4));
    }

    #[test]
    fn a_mirror_moves_when_the_selection_nears_its_edge() {
        let mut mirror = Mirror::default();
        let editable = 0..100_000;
        let window = mirror.window(&(50_000..50_000), &editable);
        assert_eq!(window, 48_000..52_000);
        mirror.sync(
            AppText {
                text: "x".repeat(4000),
                start: window.start,
                selection: 50_000..50_000,
                marked: None,
            },
            false,
        );
        mirror.reported(1, 1, state(&"x".repeat(4000), 2000..2000, None));
        assert_eq!(mirror.window(&(50_100..50_100), &editable), window);
        assert_eq!(
            mirror.window(&(51_900..51_900), &editable),
            49_900..53_900,
            "too close to the end"
        );
        assert_eq!(
            mirror.window(&(10..10), &(0..300)),
            0..300,
            "a short field is mirrored whole"
        );
    }
}
