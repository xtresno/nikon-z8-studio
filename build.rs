fn main() {
    cc::Build::new().file("csrc/jpeg.c").opt_level(3).compile("z8jpeg");
    println!("cargo:rustc-link-lib=jpeg");
    println!("cargo:rustc-link-lib=gphoto2");
    println!("cargo:rustc-link-lib=gphoto2_port");
    println!("cargo:rerun-if-changed=csrc/jpeg.c");
}
