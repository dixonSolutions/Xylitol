fn main() {
    println!("cargo:rerun-if-changed=native/log.c");
    cc::Build::new()
        .file("native/log.c")
        .warnings(true)
        .compile("xylitol_shim_native");
}
