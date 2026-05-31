fn main() {
    println!("cargo:rerun-if-changed=native/KurokoMetalDemo.m");

    cc::Build::new()
        .file("native/KurokoMetalDemo.m")
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .compile("KurokoMetalDemo");

    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Metal");
}
