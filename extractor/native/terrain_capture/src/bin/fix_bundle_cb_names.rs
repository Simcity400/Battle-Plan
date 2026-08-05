use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use terrain_capture::{
    repair_map_frame_bundle_constant_buffer_names, MAP_FRAME_BUNDLE_FILE,
    WATER_MAP_FRAME_BUNDLE_FILE,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("fix_bundle_cb_names: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_usage();
        return Ok(());
    }
    let input = args
        .first()
        .filter(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
        .ok_or("missing input session directory")?;
    let out = value_after(&mut args, "--out").ok_or("missing --out <dir>")?;
    if !input.is_dir() {
        return Err(format!(
            "input session directory does not exist: {}",
            input.display()
        )
        .into());
    }
    if out.exists() {
        return Err(format!("output directory already exists: {}", out.display()).into());
    }
    copy_dir_all(&input, &out)?;

    let mut repaired_any = false;
    for bundle_file in [MAP_FRAME_BUNDLE_FILE, WATER_MAP_FRAME_BUNDLE_FILE] {
        let bundle_path = out.join(bundle_file);
        if !bundle_path.exists() {
            continue;
        }
        let report = repair_map_frame_bundle_constant_buffer_names(&bundle_path)?;
        repaired_any |= report.changed();
        eprintln!(
            "{}: binding_names_changed={} shader_names_changed={}",
            bundle_path.display(),
            report.binding_names_changed,
            report.shader_names_changed
        );
    }
    if !repaired_any {
        eprintln!("no constant-buffer name changes were needed");
    }
    Ok(())
}

fn print_usage() {
    eprintln!("usage: fix_bundle_cb_names <session-dir> --out <fixed-session-dir>");
}

fn value_after(args: &mut Vec<String>, flag: &str) -> Option<PathBuf> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| PathBuf::from(&window[1]))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}
