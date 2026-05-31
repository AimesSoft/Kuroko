fn main() {
    cc::Build::new()
        .file("native/PresenterCheck.m")
        .flag("-fobjc-arc")
        .compile("kuroko_metal_presenter_check");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Metal");
}
