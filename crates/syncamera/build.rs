fn main() {
    println!("cargo:rerun-if-changed=src/venc.c");
    cc::Build::new().file("src/venc.c").warnings(false).compile("venc");
}
