fn main() {
    println!("cargo:rerun-if-changed=src/seh_read.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        let mut build = cc::Build::new();
        let compiler = build.get_compiler();
        if compiler.is_like_clang_cl() {
            build.flag("/EHa");
        } else if compiler.is_like_clang() {
            panic!("seh_read.c needs clang-cl, whose /EHa keeps its guarded reads handled");
        }
        build.file("src/seh_read.c").compile("seh_read");
    }
}
