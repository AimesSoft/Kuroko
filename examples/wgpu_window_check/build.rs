fn main() {
    cc::Build::new()
        .file("native/WgpuWindowCheck.m")
        .flag("-fobjc-arc")
        .compile("kuroko_wgpu_window_check");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Metal");
}
