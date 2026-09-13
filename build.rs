use std::{env, fs, path::Path, path::PathBuf, process::Command};

fn main() {
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ui_out = out.join("ui");
    fs::create_dir_all(&ui_out).unwrap();

    let mut blps: Vec<PathBuf> = fs::read_dir("data/ui")
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "blp"))
        .collect();
    blps.sort();
    let status = Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(&ui_out)
        .arg("data/ui")
        .args(&blps)
        .status()
        .expect("blueprint-compiler not found; install it (dnf install blueprint-compiler)");
    assert!(status.success(), "blueprint-compiler failed");

    // The gresource manifest names the compiled ui/*.ui next to the icons and the stylesheet.
    let resource_dir = out.join("resources");
    fs::create_dir_all(&resource_dir).unwrap();
    copy_dir("data/icons", &resource_dir.join("icons"));
    copy_dir(&ui_out, &resource_dir.join("ui"));
    fs::copy(
        "data/rill.gresource.xml",
        resource_dir.join("rill.gresource.xml"),
    )
    .unwrap();
    fs::copy("data/style.css", resource_dir.join("style.css")).unwrap();

    glib_build_tools::compile_resources(
        &[resource_dir.to_str().unwrap()],
        resource_dir.join("rill.gresource.xml").to_str().unwrap(),
        "rill.gresource",
    );

    compile_translations(&out);

    println!("cargo:rerun-if-changed=data/ui");
    println!("cargo:rerun-if-changed=data/icons");
    println!("cargo:rerun-if-changed=data/rill.gresource.xml");
    println!("cargo:rerun-if-changed=data/style.css");
    println!("cargo:rerun-if-changed=po");
}

/// Compiles every `po/<lang>.po` into `<OUT_DIR>/locale/<lang>/LC_MESSAGES/rill.mo`, so a
/// debug build run from the source tree is translated without an install. Without
/// `msgfmt` the build still succeeds, untranslated.
fn compile_translations(out: &Path) {
    let locale_dir = out.join("locale");
    let Ok(linguas) = fs::read_to_string("po/LINGUAS") else {
        return;
    };
    for lang in linguas.split_whitespace() {
        let dest = locale_dir.join(lang).join("LC_MESSAGES");
        fs::create_dir_all(&dest).unwrap();
        match Command::new("msgfmt")
            .arg(format!("po/{lang}.po"))
            .arg("-o")
            .arg(dest.join("rill.mo"))
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => println!("cargo:warning=msgfmt failed for po/{lang}.po ({status})"),
            Err(e) => {
                println!("cargo:warning=msgfmt not found, running untranslated: {e}");
                return;
            }
        }
    }
    println!(
        "cargo:rustc-env=RILL_BUILD_LOCALEDIR={}",
        locale_dir.display()
    );
}

fn copy_dir(src: impl AsRef<Path>, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for e in fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(e.path(), &to);
        } else {
            fs::copy(e.path(), to).unwrap();
        }
    }
}
