use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create terminal shim output directory");
    for entry in fs::read_dir(source).expect("read Zed terminal sources") {
        let entry = entry.expect("read Zed terminal source entry");
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if entry
            .file_type()
            .expect("inspect Zed terminal source")
            .is_dir()
        {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).expect("copy Zed terminal source");
        }
    }
}

fn apply(source: &Path, patch: &Path, destination: &Path) {
    let source = fs::read_to_string(source).expect("read Zed terminal source to patch");
    let patch_text = fs::read_to_string(patch).expect("read terminal shim patch");
    let parsed = diffy::Patch::from_str(&patch_text).expect("parse terminal shim patch");
    let mut result = diffy::apply(&source, &parsed).unwrap_or_else(|error| {
        panic!(
            "the Zed terminal source no longer matches {}: {error}; update the in-repo shim patch",
            patch.display()
        )
    });
    if destination.file_name().and_then(|name| name.to_str()) == Some("terminal.rs") {
        result = result.replacen(
            "pub mod terminal_settings;",
            "#[cfg(not(target_os = \"android\"))]\npub mod terminal_settings;\n#[cfg(target_os = \"android\")]\n#[path = \"terminal_settings_android.rs\"]\npub mod terminal_settings;",
            1,
        );
        result = result.replacen(
            "use settings::Settings;",
            "#[cfg(not(target_os = \"android\"))]\nuse settings::Settings;",
            1,
        );
    }
    fs::write(destination, result).expect("write patched terminal shim source");
}

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let zed = env::var_os("PI_ZED_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../../zed"));
    let source = zed.join("crates/terminal/src");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("src");
    let terminal_patch = manifest.join("patches/terminal.patch");
    let alacritty_patch = manifest.join("patches/alacritty.patch");

    println!("cargo:rerun-if-env-changed=PI_ZED_ROOT");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", terminal_patch.display());
    println!("cargo:rerun-if-changed={}", alacritty_patch.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest.join("src/terminal_settings_android.rs").display()
    );

    if output.exists() {
        fs::remove_dir_all(&output).expect("clear old terminal shim output");
    }
    copy_tree(&source, &output);
    apply(
        &source.join("terminal.rs"),
        &terminal_patch,
        &output.join("terminal.rs"),
    );
    apply(
        &source.join("alacritty.rs"),
        &alacritty_patch,
        &output.join("alacritty.rs"),
    );
    fs::copy(
        manifest.join("src/terminal_settings_android.rs"),
        output.join("terminal_settings_android.rs"),
    )
    .expect("copy Android terminal settings shim");
}
