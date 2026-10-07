//! Images a tool returned, such as a screenshot Pi took and read: found in the
//! tool's result, then decoded once for drawing.

use base64::Engine as _;
use gpui::{Image, ImageFormat};
use serde_json::Value;
use std::sync::Arc;

/// An image a tool returned. A durable session keeps the bytes on the
/// computer and sends `key`, its id there; others send them inline.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolImage {
    pub key: String,
    /// The file it came from, when the tool read one: "shot.png".
    pub name: String,
    pub mime: String,
    /// Base64, when the session sent the bytes.
    pub inline: Option<String>,
}

/// The images in a tool's result, named by the file it read.
pub fn tool_images(images: &[Value], path: Option<&str>) -> Vec<ToolImage> {
    let count = images.len();
    images
        .iter()
        .enumerate()
        .filter_map(|(index, image)| {
            let mime = image["mimeType"].as_str()?.to_owned();
            let data = image["data"].as_str().filter(|data| !data.is_empty());
            let key = match (image["imageId"].as_str(), data) {
                (Some(id), _) => id.to_owned(),
                (None, Some(data)) => {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    data.hash(&mut hasher);
                    format!("inline-{:016x}", hasher.finish())
                }
                (None, None) => return None,
            };
            let file = path
                .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
                .unwrap_or_else(|| "Image".into());
            Some(ToolImage {
                key,
                name: if count > 1 {
                    format!("{file} ({})", index + 1)
                } else {
                    file
                },
                mime,
                inline: data.map(str::to_owned),
            })
        })
        .collect()
}

/// A decoded image, its size in pixels, and its width over its height.
#[derive(Clone)]
pub struct Decoded {
    pub image: Arc<Image>,
    pub width: u32,
    pub height: u32,
    pub ratio: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DecodeError {
    /// A type gpui can't draw, such as `image/tiff`.
    Unsupported(String),
    Unreadable,
}

/// The bytes of an image as base64, as tools and Markdown carry them.
pub fn decode_base64(mime: &str, data: &str) -> Result<Decoded, DecodeError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| DecodeError::Unreadable)?;
    decode(mime, bytes)
}

pub fn decode(mime: &str, bytes: Vec<u8>) -> Result<Decoded, DecodeError> {
    let format = ImageFormat::from_mime_type(mime)
        .ok_or_else(|| DecodeError::Unsupported(mime.to_owned()))?;
    decode_as(format, bytes)
}

/// Bytes known to be `format`, with their size: an SVG's from its own
/// `width`, `height` or `viewBox`, as gpui will draw it.
pub(crate) fn decode_as(format: ImageFormat, bytes: Vec<u8>) -> Result<Decoded, DecodeError> {
    let (width, height) = if format == ImageFormat::Svg {
        let tree = usvg::Tree::from_data(&bytes, &usvg::Options::default())
            .map_err(|_| DecodeError::Unreadable)?;
        let size = tree.size();
        (size.width().ceil() as u32, size.height().ceil() as u32)
    } else {
        image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .ok()
            .and_then(|reader| reader.into_dimensions().ok())
            .ok_or(DecodeError::Unreadable)?
    };
    Ok(Decoded {
        image: Arc::new(Image::from_bytes(format, bytes)),
        width,
        height,
        ratio: width.max(1) as f32 / height.max(1) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn images_are_named_by_the_file_and_keyed_by_id_or_bytes() {
        let images = [
            json!({"type":"image","data":"","imageId":"abc","mimeType":"image/png"}),
            json!({"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"}),
            json!({"type":"image","data":"","mimeType":"image/png"}),
        ];
        let found = tool_images(&images, Some("/tmp/shots/page.png"));
        assert_eq!(
            found.len(),
            2,
            "an image with neither id nor bytes is skipped"
        );
        assert_eq!(
            (found[0].key.as_str(), found[0].name.as_str()),
            ("abc", "page.png (1)")
        );
        assert_eq!(found[0].inline, None);
        assert!(found[1].key.starts_with("inline-"));
        assert_eq!(found[1].inline.as_deref(), Some("iVBORw0KGgo="));
        assert_eq!(tool_images(&images[..1], None)[0].name, "Image");
    }

    #[test]
    fn decoding_gives_the_shape_or_says_why_not() {
        let png = crate::sample::tool_png();
        let decoded = decode_base64("image/png", &png).unwrap();
        assert_eq!(
            (decoded.width, decoded.height, decoded.ratio),
            (320, 160, 2.)
        );
        assert_eq!(
            decode_base64("image/x-unknown", &png).err(),
            Some(DecodeError::Unsupported("image/x-unknown".into()))
        );
        assert_eq!(
            decode_base64("image/png", "bm90IGFuIGltYWdl").err(),
            Some(DecodeError::Unreadable)
        );
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 120 40"/>"#.to_vec();
        let decoded = decode("image/svg+xml", svg).unwrap();
        assert_eq!(
            (decoded.width, decoded.height, decoded.ratio),
            (120, 40, 3.)
        );
    }
}
