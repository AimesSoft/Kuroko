use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=ERIKA_NATIVE_PROFILE");
    println!("cargo:rerun-if-env-changed=ERIKA_NATIVE_TARGET");
    println!("cargo:rerun-if-env-changed=ERIKA_LIBASS_DIR");
    println!("cargo:rerun-if-env-changed=ERIKA_FREETYPE_DIR");
    println!("cargo:rerun-if-env-changed=ERIKA_HARFBUZZ_DIR");
    println!("cargo:rerun-if-env-changed=ERIKA_FRIBIDI_DIR");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rustc-link-lib=framework=AudioToolbox");
    }

    if env::var("CARGO_FEATURE_LIBASS").is_err() {
        return;
    }

    let libass = native_dep_dir("ERIKA_LIBASS_DIR", "libass");
    let freetype = native_dep_dir("ERIKA_FREETYPE_DIR", "freetype");
    let harfbuzz = native_dep_dir("ERIKA_HARFBUZZ_DIR", "harfbuzz");
    let fribidi = native_dep_dir("ERIKA_FRIBIDI_DIR", "fribidi");

    for dir in [&libass, &freetype, &harfbuzz, &fribidi] {
        if !dir.join("lib").exists() {
            panic!(
                "native dependency was not found at {}. Run `cargo run -p xtask -- deps build --all --profile {}` first, or set ERIKA_*_DIR.",
                dir.display(),
                native_profile()
            );
        }
        println!(
            "cargo:rustc-link-search=native={}",
            dir.join("lib").display()
        );
    }

    if !libass.join("include/ass/ass.h").exists() && !libass.join("include/ass.h").exists() {
        panic!(
            "libass headers were not found under {}. Run `cargo run -p xtask -- deps build --all --profile {}` first.",
            libass.display(),
            native_profile()
        );
    }

    println!("cargo:rustc-link-lib=static=ass");
    println!("cargo:rustc-link-lib=static=fribidi");
    println!("cargo:rustc-link-lib=static=harfbuzz");
    println!("cargo:rustc-link-lib=static=freetype");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=CoreText");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=iconv");
    }
}

fn native_dep_dir(env_name: &str, name: &str) -> PathBuf {
    if let Ok(path) = env::var(env_name) {
        return PathBuf::from(path);
    }
    if let Ok(target) = env::var("ERIKA_NATIVE_TARGET") {
        return workspace_root()
            .join("third_party/dist")
            .join(target)
            .join(native_profile())
            .join(name);
    }
    workspace_root()
        .join("third_party/dist")
        .join(native_profile())
        .join(name)
}

fn native_profile() -> String {
    env::var("ERIKA_NATIVE_PROFILE").unwrap_or_else(|_| "lgpl".to_string())
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crates/erika has a workspace root")
        .to_path_buf()
}
