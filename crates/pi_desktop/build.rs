//! Embeds the app icon in the Windows executable. Other platforms use
//! packaging/macos/AppIcon.icns and the PNGs in assets/app-icon. With the
//! `bundled-backend` feature, also names the embedded pi archive.

fn main() {
    if std::env::var_os("CARGO_FEATURE_BUNDLED_BACKEND").is_some() {
        bundled_backend();
    }
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("packaging")
        .join("windows")
        .join("app-icon.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    #[cfg(windows)]
    {
        // rc.exe resolves relative paths against its own working directory, so the
        // generated script names the icon by its absolute path.
        let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
        let rc = out.join("pi-desktop.rc");
        let escaped = icon.display().to_string().replace('\\', "\\\\");
        std::fs::write(&rc, format!("1 ICON \"{escaped}\"\n")).expect("writing pi-desktop.rc");
        embed_resource::compile(&rc, embed_resource::NONE)
            .manifest_optional()
            .expect("embedding the Windows app icon");
    }
}

/// Points `include_bytes!` at PI_DESKTOP_BACKEND_ARCHIVE and names this archive
/// by a hash of it, so each different pi unpacks into its own folder.
fn bundled_backend() {
    use std::hash::{Hash, Hasher};
    println!("cargo:rerun-if-env-changed=PI_DESKTOP_BACKEND_ARCHIVE");
    let archive = std::env::var_os("PI_DESKTOP_BACKEND_ARCHIVE").expect(
        "the bundled-backend feature needs PI_DESKTOP_BACKEND_ARCHIVE: the .tar.zst of pi's \
         release that scripts/fetch_pi.py writes",
    );
    let archive = std::path::absolute(archive).expect("PI_DESKTOP_BACKEND_ARCHIVE");
    println!("cargo:rerun-if-changed={}", archive.display());
    let bytes = std::fs::read(&archive).expect("reading PI_DESKTOP_BACKEND_ARCHIVE");
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    println!(
        "cargo:rustc-env=PI_DESKTOP_BACKEND_ARCHIVE={}",
        archive.display()
    );
    println!(
        "cargo:rustc-env=PI_DESKTOP_BACKEND_ID={:016x}",
        hasher.finish()
    );
}
