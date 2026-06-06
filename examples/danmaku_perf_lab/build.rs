fn main() {
    println!("cargo:rerun-if-changed=native/DanmakuPerfLab.m");

    cc::Build::new()
        .file("native/DanmakuPerfLab.m")
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .compile("KurokoDanmakuPerfLab");

    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Metal");
}
