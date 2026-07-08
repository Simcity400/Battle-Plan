use game_ui::probe::{default_probe_config, run_probe};

fn main() {
    let mut config = default_probe_config();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--game-root" => {
                let Some(value) = args.next() else {
                    eprintln!("--game-root needs a path");
                    std::process::exit(2);
                };
                config.vanilla_root = value.into();
            }
            "--mod-root" => {
                let Some(value) = args.next() else {
                    eprintln!("--mod-root needs a path");
                    std::process::exit(2);
                };
                config.mod_root = Some(value.into());
            }
            "--out" => {
                let Some(value) = args.next() else {
                    eprintln!("--out needs a path");
                    std::process::exit(2);
                };
                config.out_dir = value.into();
            }
            "--no-png" => config.write_pngs = false,
            _ => {
                eprintln!("unknown argument: {arg}");
                std::process::exit(2);
            }
        }
    }

    match run_probe(&config) {
        Ok(report) => println!("{}", report.to_text()),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}
