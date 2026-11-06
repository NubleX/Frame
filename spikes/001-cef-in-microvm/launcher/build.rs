fn main() {
    if let Ok(dir) = std::env::var("KRUN_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
    }
    println!("cargo:rustc-link-lib=dylib=krun");
    println!("cargo:rerun-if-env-changed=KRUN_LIB_DIR");
}
