use std::path::PathBuf;

use render::visual_diff::diff_png_files;

fn usage(program: &str) -> ! {
    eprintln!("Usage: {program} <left.png> <right.png> <diff.png>");
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "png_visual_diff".to_string());
    let Some(left) = args.next() else {
        usage(&program);
    };
    let Some(right) = args.next() else {
        usage(&program);
    };
    let Some(diff) = args.next() else {
        usage(&program);
    };
    if args.next().is_some() {
        usage(&program);
    }

    let left = PathBuf::from(left);
    let right = PathBuf::from(right);
    let diff = PathBuf::from(diff);
    match diff_png_files(&left, &right, &diff) {
        Ok(report) => {
            let json = report.to_json_value(&left, &right, &diff);
            println!(
                "{}",
                serde_json::to_string_pretty(&json).expect("visual diff metadata JSON")
            );
        }
        Err(e) => {
            eprintln!("[visual-diff] FAIL-CLOSED: {e}");
            std::process::exit(2);
        }
    }
}
