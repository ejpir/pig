//! A page's poster: the top of an HTML page as it looks, for its card in the
//! thread. On macOS the system's WebKit draws it offscreen, running the
//! page's scripts as Safari would, and it is kept as a PNG by the page's
//! contents. Elsewhere a page card has no poster.

use std::path::{Path, PathBuf};

/// The page's viewport, in points; the poster is its top.
pub const WIDTH: f64 = 1280.;
pub const HEIGHT: f64 = 560.;

/// Where the poster of a page with this content is kept.
pub fn path(html: &str, page: &str) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (html, page).hash(&mut hasher);
    Some(
        dirs::cache_dir()?
            .join("pi-desktop")
            .join("posters")
            .join(format!("{:016x}.png", hasher.finish())),
    )
}

/// A page being drawn: load it, wait while [`Drawing::loading`], then take
/// the [`Drawing::snapshot`].
#[cfg(target_os = "macos")]
pub struct Drawing(objc2::rc::Retained<objc2_web_kit::WKWebView>);

#[cfg(target_os = "macos")]
impl Drawing {
    /// Starts loading `file`, which may read files next to it in `folder`.
    /// `None` off the main thread or when WebKit refuses the file.
    pub fn start(file: &Path, folder: &Path, html: &str) -> Option<Self> {
        use objc2::{MainThreadMarker, MainThreadOnly as _};
        use objc2_core_foundation::{CGPoint, CGRect, CGSize};
        use objc2_foundation::{NSString, NSURL};
        use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
        let main = MainThreadMarker::new()?;
        let (file, folder) = (
            NSURL::from_file_path(file)?,
            NSURL::from_directory_path(folder)?,
        );
        unsafe {
            let configuration = WKWebViewConfiguration::new(main);
            let view = WKWebView::initWithFrame_configuration(
                WKWebView::alloc(main),
                CGRect::new(CGPoint::new(0., 0.), CGSize::new(WIDTH, HEIGHT)),
                &configuration,
            );
            // WebKit reads a file that doesn't name its encoding as Latin-1;
            // given as text, the page is read as the UTF-8 Pi wrote.
            if utf8_unlabelled(html) {
                view.loadHTMLString_baseURL(&NSString::from_str(html), Some(&folder))?;
            } else {
                view.loadFileURL_allowingReadAccessToURL(&file, &folder)?;
            }
            Some(Self(view))
        }
    }

    pub fn loading(&self) -> bool {
        unsafe { self.0.isLoading() }
    }

    /// Saves what the page shows now to `out` as a PNG, then calls `done`
    /// with whether it worked.
    pub fn snapshot(self, out: PathBuf, done: impl FnOnce(bool) + 'static) {
        use objc2::{AnyThread as _, MainThreadOnly as _};
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage};
        use objc2_foundation::{NSDictionary, NSError, NSNumber};
        use objc2_web_kit::WKSnapshotConfiguration;
        let done = std::cell::Cell::new(Some(done));
        let view = self.0.clone();
        let handler = block2::RcBlock::new(move |image: *mut NSImage, _: *mut NSError| {
            // Keeps the view alive until WebKit has answered.
            let _ = &view;
            let saved = unsafe { image.as_ref() }
                .and_then(|image| image.TIFFRepresentation())
                .and_then(|tiff| NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &tiff))
                .and_then(|bitmap| unsafe {
                    bitmap.representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                })
                .is_some_and(|png| {
                    out.parent()
                        .is_some_and(|folder| std::fs::create_dir_all(folder).is_ok())
                        && std::fs::write(&out, png.to_vec()).is_ok()
                });
            if let Some(done) = done.take() {
                done(saved);
            }
        });
        unsafe {
            let configuration = WKSnapshotConfiguration::new(self.0.mtm());
            // Half the viewport's width: sharp on a Retina card, and small.
            configuration.setSnapshotWidth(Some(&NSNumber::numberWithDouble(WIDTH / 2.)));
            self.0
                .takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &handler);
        }
    }
}

/// Whether WebKit would misread the page: not plain ASCII, and no `charset`.
pub fn utf8_unlabelled(html: &str) -> bool {
    !html.is_ascii() && !html.to_ascii_lowercase().contains("charset")
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_unlabelled_pages_outside_ascii_are_given_as_text() {
        assert!(super::utf8_unlabelled("<h1>Café</h1>"));
        assert!(!super::utf8_unlabelled(
            "<meta charset=\"utf-8\"><h1>Café</h1>"
        ));
        assert!(!super::utf8_unlabelled("<h1>Cafe</h1>"));
    }
}
