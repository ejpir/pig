//! Small, offline QR decoder shared by the Android JNI bridge and host tests.

use std::cell::RefCell;

thread_local! {
    // Camera frames arrive on one background thread. Reusing Quirc avoids a
    // fresh image-buffer allocation every 140 ms while keeping it off the UI.
    static DECODER: RefCell<quircs::Quirc> = RefCell::new(quircs::Quirc::default());
}

pub fn decode(width: usize, height: usize, image: &mut [u8]) -> Option<String> {
    if width == 0 || height == 0 || image.len() != width.checked_mul(height)? {
        return None;
    }
    if let Some(decoded) = decode_polarity(width, height, image) {
        return Some(decoded);
    }
    // Dark terminals display Unicode block glyphs as a light-on-dark QR.
    // Accept that polarity too, without another camera-sized allocation.
    for pixel in image.iter_mut() {
        *pixel = 255 - *pixel;
    }
    decode_polarity(width, height, image)
}

fn decode_polarity(width: usize, height: usize, image: &[u8]) -> Option<String> {
    DECODER.with(|decoder| {
        let mut decoder = decoder.borrow_mut();
        for code in decoder.identify(width, height, image).flatten() {
            let Ok(decoded) = code.decode() else { continue };
            let Ok(text) = std::str::from_utf8(&decoded.payload) else {
                continue;
            };
            return Some(text.to_owned());
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qrcode::{QrCode, types::Color};

    #[test]
    fn decodes_the_pi_url_from_a_camera_like_luminance_frame() {
        let expected = "pi://pair/v1#one-use-offer";
        let code = QrCode::new(expected).unwrap();
        let modules = code.width();
        let (quiet, scale) = (4, 6);
        let side = (modules + quiet * 2) * scale;
        let mut frame = vec![255; side * side];
        for y in 0..modules {
            for x in 0..modules {
                if code[(x, y)] != Color::Dark {
                    continue;
                }
                for row in 0..scale {
                    let start = ((y + quiet) * scale + row) * side + (x + quiet) * scale;
                    frame[start..start + scale].fill(0);
                }
            }
        }
        assert_eq!(decode(side, side, &mut frame).as_deref(), Some(expected));
        for pixel in &mut frame {
            *pixel = 255 - *pixel;
        }
        assert_eq!(
            decode(side, side, &mut frame).as_deref(),
            Some(expected),
            "dark-terminal polarity is accepted"
        );
    }
}
