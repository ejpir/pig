//! The shared sample (`pi_markdown::sample`) drawn on desktop: each kind of
//! picture and page in it, as Android shows it too.
use super::*;
use markdown::parser::{CodeBlockKind, MarkdownEvent, MarkdownTag};
use pi_markdown::sample::{self, Item};

#[gpui::test]
fn the_shared_sample_shows_every_picture_and_page(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    // Tall enough for the whole sample at once.
    cx.simulate_resize(size(px(1344.), px(1600.)));
    let session = tab(&desktop, 0, &cx);
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    receive(&session, sample::record(), &mut cx);
    cx.run_until_parked();
    // Pi's tool results go into its tools, so the reply's place is the model's.
    let reply = session.controller.read_with(&cx, |controller, _| {
        let messages = &controller.model().messages;
        messages
            .iter()
            .rposition(|message| message["role"] == "assistant")
            .unwrap()
    });
    let prose: &'static str = format!("assistant-prose-{reply}-0").leak();
    assert!(cx.debug_bounds(prose).is_some());
    let document = session.transcript.read_with(&cx, |view, _| {
        view.documents
            .get(&format!("{reply}:text-0"))
            .unwrap()
            .clone()
    });
    let events = document.read_with(&cx, |document, _| document.parsed_markdown().events.clone());
    let image = |kind: &str| {
        events.iter().any(|(_, event)| {
            matches!(event, MarkdownEvent::Start(MarkdownTag::Image { dest_url, .. })
                if dest_url.starts_with(&format!("data:{kind};base64,")))
        })
    };
    for item in sample::ITEMS {
        match item {
            Item::InlineSvg | Item::SvgBlock | Item::SvgMarkup => {
                let svgs = events
                    .iter()
                    .filter(|(_, event)| {
                        matches!(event, MarkdownEvent::Start(MarkdownTag::Image { dest_url, .. })
                            if dest_url.starts_with("data:image/svg+xml;base64,"))
                    })
                    .count();
                assert!(sample::svg_order(item).unwrap() < svgs, "{item:?}");
            }
            Item::InlinePng => assert!(image("image/png"), "{item:?}"),
            Item::Mermaid => assert!(
                events.iter().any(|(_, event)| matches!(
                    event,
                    MarkdownEvent::Start(MarkdownTag::CodeBlock { kind: CodeBlockKind::FencedLang(language), .. })
                        if language.as_ref() == "mermaid"
                )),
                "{item:?}"
            ),
            Item::ToolImage => {
                // The read's PNG is 320 by 160, drawn at its own size above
                // its name; while it loads, the card is shorter.
                let card = cx.debug_bounds("tool-image-shot-0").expect("tool image");
                assert!(card.size.height > px(180.), "{item:?} is drawn: {card:?}");
            }
            Item::Page => {
                // Written and then edited in one turn: one card, at the edit.
                assert!(cx.debug_bounds("page-card-tweak").is_some(), "{item:?}");
                assert!(cx.debug_bounds("page-card-page").is_none());
                assert!(cx.debug_bounds("open-page-tweak").is_some());
            }
        }
    }
}

#[gpui::test]
fn an_image_kept_on_the_computer_shows_once_it_arrives(cx: &mut TestAppContext) {
    let (desktop, mut cx) = setup(cx);
    let session = tab(&desktop, 0, &cx);
    receive(&session, json!({"type":"agent_settled"}), &mut cx);
    let mut record = sample::record();
    let shot = record["data"]["messages"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|message| message["toolCallId"] == "shot")
        .unwrap();
    let data = shot["content"][1]["data"].take();
    shot["content"][1]["imageId"] = json!("kept");
    shot["content"][1]["data"] = json!("");
    receive(&session, record, &mut cx);
    cx.run_until_parked();
    let waiting = cx.debug_bounds("tool-image-shot-0").unwrap();
    session.controller.update(&mut cx, |controller, _| {
        controller.awaiting_image("ask", "kept")
    });
    receive(
        &session,
        json!({"type":"response","command":"get_image","id":"ask","success":true,
            "data":{"image":{"mimeType":"image/png","data":data}}}),
        &mut cx,
    );
    cx.run_until_parked();
    let shown = cx.debug_bounds("tool-image-shot-0").unwrap();
    assert!(
        shown.size.height > waiting.size.height,
        "{waiting:?} → {shown:?}"
    );
}
