fn main() {
    println!("cargo:rerun-if-changed=native/ErikaMetalDemo.m");

    cc::Build::new()
        .file("native/ErikaMetalDemo.m")
        .flag("-fobjc-arc")
        .flag("-fmodules")
        .compile("ErikaMetalDemo");

    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
    println!("cargo:rustc-link-lib=framework=Metal");
}
