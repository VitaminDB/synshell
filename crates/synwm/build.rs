fn main() {
    println!("cargo:rerun-if-changed=src/v4l2enc.c");
    cc::Build::new().file("src/v4l2enc.c").warnings(false).compile("v4l2enc");
}
