use pi_core::skill::{SkillBlock, parse_skill_block};

const BLOCK: &str = "<skill name=\"release\" location=\"/repo/.pi/skills/release/SKILL.md\">\nReferences are relative to /repo/.pi/skills/release.\n\n# Release\n\nRun `npm run check`.\n</skill>";

#[test]
fn skill_block_without_arguments() {
    assert_eq!(
        parse_skill_block(BLOCK),
        Some(SkillBlock {
            name: "release",
            location: "/repo/.pi/skills/release/SKILL.md",
            content: "References are relative to /repo/.pi/skills/release.\n\n# Release\n\nRun `npm run check`.",
            user_message: None,
        })
    );
}

#[test]
fn skill_block_with_arguments_trims_them() {
    let text = format!("{BLOCK}\n\n  cut 0.88.0\n");
    let block = parse_skill_block(&text).unwrap();
    assert_eq!(block.name, "release");
    assert_eq!(block.user_message, Some("cut 0.88.0"));
    assert!(block.content.ends_with("Run `npm run check`."));
}

#[test]
fn body_ends_at_the_first_closing_tag_the_tail_can_follow() {
    let text =
        "<skill name=\"a\" location=\"/a.md\">\nfirst\n</skill>\nnot a tail\n</skill>\n\nargs";
    let block = parse_skill_block(text).unwrap();
    assert_eq!(block.content, "first\n</skill>\nnot a tail");
    assert_eq!(block.user_message, Some("args"));
}

#[test]
fn ordinary_messages_are_not_skill_blocks() {
    for text in [
        "hello",
        "<skill name=\"\" location=\"/a.md\">\nbody\n</skill>",
        "<skill name=\"a\" location=\"/a.md\">body\n</skill>",
        "<skill name=\"a\" location=\"/a.md\">\nbody\n</skill>trailing",
        "<skill name=\"a\" location=\"/a.md\">\nbody\n</skill>\n\n",
        "prefix <skill name=\"a\" location=\"/a.md\">\nbody\n</skill>",
    ] {
        assert_eq!(parse_skill_block(text), None, "{text:?}");
    }
}

#[test]
fn whitespace_only_arguments_are_omitted() {
    let text = format!("{BLOCK}\n\n   ");
    assert_eq!(parse_skill_block(&text).unwrap().user_message, None);
}
