use std::env;
use std::path::Path;

fn main() {
    let root_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let target = env::var("TARGET").unwrap();

    let parts = target.splitn(4, '-').collect::<Vec<_>>();
    let arch = parts[0];
    let sys = parts[2];

    if sys != "windows" {
        panic!("Platform '{sys}' not supported.");
    }

    let hde = match arch {
        "i686" => "hde/hde32.c",
        "x86_64" => "hde/hde64.c",
        _ => panic!("Architecture '{arch}' not supported."),
    };

    let src_dir = Path::new(&root_dir).join("vendor/src");

    cc::Build::new()
        .file(src_dir.join("buffer.c"))
        .file(src_dir.join("hook.c"))
        .file(src_dir.join("trampoline.c"))
        .file(src_dir.join(hde))
        .compile("libminhook_h4.a");

    println!("cargo:rerun-if-changed=vendor");
    println!(
        "cargo:rustc-link-search=native={}",
        env::var("OUT_DIR").unwrap()
    );
}
