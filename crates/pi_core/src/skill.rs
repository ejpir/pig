/// A `/skill:name args` prompt after pi expanded it into the user message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillBlock<'a> {
    pub name: &'a str,
    pub location: &'a str,
    pub content: &'a str,
    /// Text typed after the command, if any.
    pub user_message: Option<&'a str>,
}

/// Same grammar as pi's `parseSkillBlock` in `core/agent-session.ts`:
/// `^<skill name="([^"]+)" location="([^"]+)">\n([\s\S]*?)\n</skill>(?:\n\n([\s\S]+))?$`
pub fn parse_skill_block(text: &str) -> Option<SkillBlock<'_>> {
    let rest = text.strip_prefix("<skill name=\"")?;
    let (name, rest) = rest.split_once('"')?;
    let rest = rest.strip_prefix(" location=\"")?;
    let (location, rest) = rest.split_once('"')?;
    let body = rest.strip_prefix(">\n")?;
    if name.is_empty() || location.is_empty() {
        return None;
    }
    // The lazy body match ends at the first closing tag that the optional tail can follow.
    body.match_indices("\n</skill>").find_map(|(end, closing)| {
        let tail = &body[end + closing.len()..];
        let user_message = if tail.is_empty() {
            None
        } else {
            let message = tail
                .strip_prefix("\n\n")
                .filter(|message| !message.is_empty())?;
            Some(message.trim()).filter(|message| !message.is_empty())
        };
        Some(SkillBlock {
            name,
            location,
            content: &body[..end],
            user_message,
        })
    })
}
