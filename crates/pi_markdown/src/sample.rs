//! One session with each kind of thing Pi can show besides text, for both apps'
//! tests. Each app renders [`RECORD`] and checks every [`Item`] it shows; the
//! checks `match` on `Item`, so a new kind added here fails to compile in an
//! app until that app handles it.

/// A `get_messages` response, as either app receives it from Pi.
pub const RECORD: &str = include_str!("../fixtures/media.json");

/// The session's folder.
pub const CWD: &str = "/repo";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    /// `![…](data:image/svg+xml;base64,…)` in the reply.
    InlineSvg,
    /// `![…](data:image/png;base64,…)` in the reply.
    InlinePng,
    /// A ```` ```svg ```` block in the reply, as Pi wrote it: no base64.
    SvgBlock,
    /// `<svg>…</svg>` markup on its own in the reply, without `xmlns`.
    SvgMarkup,
    /// A ```` ```mermaid ```` block in the reply.
    Mermaid,
    /// The PNG `read` returned for `shot.png`.
    ToolImage,
    /// `sample/page.html`, written and then edited from "blue" to "green".
    Page,
}

pub const ITEMS: [Item; 7] = [
    Item::InlineSvg,
    Item::InlinePng,
    Item::SvgBlock,
    Item::SvgMarkup,
    Item::Mermaid,
    Item::ToolImage,
    Item::Page,
];

pub const TOOL_IMAGE_NAME: &str = "shot.png";
pub const PAGE_PATH: &str = "sample/page.html";
pub const PAGE_TITLE: &str = "Sample page";
/// The page's body after the edit; before it, the page said "blue".
pub const PAGE_BODY: &str = "<p>green</p>";

pub fn record() -> serde_json::Value {
    serde_json::from_str(RECORD).expect("the sample is JSON")
}

/// Which of the reply's SVG pictures an item is, in reading order.
pub fn svg_order(item: Item) -> Option<usize> {
    match item {
        Item::InlineSvg => Some(0),
        Item::SvgBlock => Some(1),
        Item::SvgMarkup => Some(2),
        _ => None,
    }
}

/// The base64 PNG the `read` returned: a 320 by 160 screenshot.
#[cfg(test)]
pub(crate) fn tool_png() -> String {
    record()["data"]["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|message| message["content"].as_array().into_iter().flatten())
        .find(|block| block["type"] == "image")
        .and_then(|block| block["data"].as_str())
        .expect("the sample has a tool image")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Media, Pages, blocks, decode_base64, tool_images};
    use serde_json::Value;

    /// The sample holds what its `Item`s promise, before any app draws it.
    #[test]
    fn the_sample_has_every_item() {
        let record = record();
        let messages = record["data"]["messages"].as_array().unwrap();
        let reply = messages
            .iter()
            .rev()
            .find(|message| message["role"] == "assistant")
            .and_then(|message| message["content"][0]["text"].as_str())
            .unwrap();
        let reply = blocks(reply);
        let svgs = reply
            .iter()
            .filter(|block| {
                matches!(&block.media, Some(Media::Image(image)) if image.format == gpui::ImageFormat::Svg)
            })
            .count();
        let images: Vec<_> = reply
            .iter()
            .filter_map(|block| match &block.media {
                Some(Media::Image(image)) => Some(image.format),
                _ => None,
            })
            .collect();
        let calls: Vec<&Value> = messages
            .iter()
            .filter(|message| message["role"] == "assistant")
            .flat_map(|message| message["content"].as_array().unwrap())
            .filter(|block| block["type"] == "toolCall")
            .collect();
        let mut pages = Pages::default();
        let mut page = None;
        for call in &calls {
            let path = call["arguments"]["path"].as_str().unwrap();
            let path = path.strip_prefix(&format!("{CWD}/")).unwrap();
            page = pages
                .follow(call["name"].as_str().unwrap(), &call["arguments"], path)
                .or(page);
        }
        for item in ITEMS {
            match item {
                Item::InlineSvg | Item::SvgBlock | Item::SvgMarkup => {
                    assert!(svg_order(item).unwrap() < svgs, "{item:?}")
                }
                Item::InlinePng => assert!(images.contains(&gpui::ImageFormat::Png)),
                Item::Mermaid => assert!(
                    reply
                        .iter()
                        .any(|block| matches!(block.media, Some(Media::Mermaid(_))))
                ),
                Item::ToolImage => {
                    let result = messages
                        .iter()
                        .find(|message| message["toolCallId"] == "shot")
                        .unwrap();
                    let images: Vec<Value> = result["content"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter(|block| block["type"] == "image")
                        .cloned()
                        .collect();
                    let found = tool_images(&images, Some("/repo/sample/shot.png"));
                    assert_eq!(found[0].name, TOOL_IMAGE_NAME);
                    let data = found[0].inline.as_deref().unwrap();
                    assert!(decode_base64(&found[0].mime, data).is_ok());
                }
                Item::Page => {
                    let page = page.clone().unwrap();
                    assert_eq!(
                        (page.path.as_str(), page.title()),
                        (PAGE_PATH, PAGE_TITLE.into())
                    );
                    assert!(page.html.unwrap().contains(PAGE_BODY));
                }
            }
        }
    }
}
