fn main() {
    println!("cargo:rerun-if-changed=src/v4l2enc.c");
    println!("cargo:rerun-if-changed=src/ffenc.c");
    cc::Build::new().file("src/v4l2enc.c").warnings(false).compile("v4l2enc");
    // Кодер компьютера (VAAPI/NVENC через libavcodec): заголовки ffmpeg нужны при
    // сборке, библиотеки грузятся dlopen'ом при первом открытии кодера.
    cc::Build::new().file("src/ffenc.c").warnings(false).compile("ffenc");
    println!("cargo:rustc-link-lib=dl");
}
