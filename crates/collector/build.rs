fn main() {
    println!("cargo:rerun-if-changed=src/event_feed_native_guard.cpp");
    println!("cargo:rerun-if-changed=src/event_feed_native_guard.h");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        cc::Build::new()
            .cpp(true)
            .file("src/event_feed_native_guard.cpp")
            .compile("event_feed_native_guard");
    }
}
