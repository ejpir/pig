//! Import actual bytes off the UI thread, with bounded decoding and upload size.
use crate::{app::size_label, composer::Attachment};
use gpui::{Image, ImageFormat};
use image::{ImageDecoder, ImageReader};
use std::{
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

pub const MAX_IMAGES: usize = 4;
pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 256 * 1024;

pub fn load(path: &Path) -> Result<Attachment, String> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_SOURCE_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|error| format!("Could not read {name}: {error}"))?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(format!("{name} exceeds the 20 MB import limit."));
    }
    if image::guess_format(&bytes).is_ok() {
        return prepare_image(name, &bytes);
    }
    if bytes.len() > MAX_TEXT_BYTES {
        return Err(format!("Text files must be at most 256 KB: {name}"));
    }
    let contents = String::from_utf8(bytes)
        .map_err(|_| format!("{name} is not a supported image or UTF-8 text file."))?;
    if contents.contains('\0') {
        return Err(format!(
            "{name} is a binary file; choose an image or text file."
        ));
    }
    Ok(Attachment::File {
        name,
        size: size_label(contents.len() as u64),
        contents,
    })
}

pub fn prepare_image(name: String, bytes: &[u8]) -> Result<Attachment, String> {
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err("Images must be at most 20 MB before resizing.".into());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "This image could not be decoded. Try PNG, JPEG, WebP or GIF.".to_owned())?;
    let orientation = decoder.orientation().map_err(|error| error.to_string())?;
    let mut decoded =
        image::DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    decoded.apply_orientation(orientation);
    let alpha = decoded.color().has_alpha();
    let mut edge = 2048;
    loop {
        let resized = decoded.thumbnail(edge, edge);
        let mut output = Cursor::new(Vec::new());
        let format = if alpha {
            ImageFormat::Png
        } else {
            ImageFormat::Jpeg
        };
        if alpha {
            resized
                .write_to(&mut output, image::ImageFormat::Png)
                .map_err(|error| error.to_string())?;
        } else {
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 85)
                .encode_image(&resized)
                .map_err(|error| error.to_string())?;
        }
        let bytes = output.into_inner();
        if bytes.len() <= MAX_IMAGE_BYTES {
            return Ok(Attachment::Image {
                name,
                image: Arc::new(Image::from_bytes(format, bytes)),
            });
        }
        if edge <= 512 {
            return Err("Could not fit this image within the 1 MB upload limit.".into());
        }
        edge = edge * 3 / 4;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uploads_have_decodable_image_bytes_and_bounded_size() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2500, 3000)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let Attachment::Image { image, .. } =
            prepare_image("photo.png".into(), &png.into_inner()).unwrap()
        else {
            panic!()
        };
        assert!(image.bytes.len() <= MAX_IMAGE_BYTES);
        let decoded = image::load_from_memory(&image.bytes).unwrap();
        assert!(decoded.width() <= 2048 && decoded.height() <= 2048);
        assert!(prepare_image("bad.png".into(), b"not a photo").is_err());
    }
}
