fn main() {
    println!("cargo:rerun-if-changed=src/seh_read.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        cc::Build::new().file("src/seh_read.c").compile("seh_read");
    }
}
