
use dashboard_base::{Source, SourceKey, catalogue, config::ContentOptions};
use std::{io, path::PathBuf, sync::atomic::AtomicBool, time::Instant};

#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

const USAGE: &str =
    "catalogue_cold --recording R --game G --user U --cache C [--samples N] [--purge-kept]";

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let (mut recording, mut game, mut user, mut cache) = (None, None, None, None);
    let (mut samples, mut purge_kept) = (5_usize, false);
    while let Some(flag) = args.next() {
        if flag == "--purge-kept" {
            purge_kept = true;
            continue;
        }
        let value = args.next().ok_or_else(|| io::Error::other(USAGE))?;
        match flag.to_str() {
            Some("--recording") => recording = Some(PathBuf::from(value)),
            Some("--game") => game = Some(PathBuf::from(value)),
            Some("--user") => user = Some(PathBuf::from(value)),
            Some("--cache") => cache = Some(PathBuf::from(value)),
            Some("--samples") => {
                samples = value.to_string_lossy().parse().map_err(io::Error::other)?;
            }
            _ => return Err(io::Error::other(USAGE)),
        }
    }
    let missing = || io::Error::other(USAGE);
    let recording = recording.ok_or_else(missing)?;
    let options = ContentOptions {
        options: content_reader_layers::ResolveOptions {
            game_dir: game.ok_or_else(missing)?,
            user_dir: user.ok_or_else(missing)?,
            cache_dir: cache.ok_or_else(missing)?,
        },
        allow_unverified: false,
        presentation: Default::default(),
    };
    std::fs::create_dir_all(&options.options.cache_dir)?;
    let kept_folder = options.options.cache_dir.join("catalogue");
    let kept_files = || -> io::Result<Vec<PathBuf>> {
        let entries = match std::fs::read_dir(&kept_folder) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error),
        };
        let mut files = Vec::new();
        for entry in entries {
            let path = entry?.path();
            if path.extension().is_some_and(|extension| extension == "bin") {
                files.push(path);
            }
        }
        Ok(files)
    };
    let cancelled = AtomicBool::new(false);
    let open = || -> io::Result<(Source, replay_tools::cold_cache::Probe)> {
        let probe = replay_tools::cold_cache::probe_file(
            &recording,
            replay_tools::cold_cache::ResetScope::FreshSourcePerSample,
        )?;
        let source = Source::open(
            SourceKey {
                source_id: "catalogue-cold".into(),
                revision: dashboard_base::protocol::replay_revision(&recording)?,
            },
            recording.clone(),
            &cancelled,
        )?;
        Ok((source, probe))
    };

    let before = kept_files()?.len();
    let (source, cache) = open()?;
    println!("cache state: {} ({})", cache.label, cache.reason);
    if purge_kept {
        println!(
            "kept catalogue files (--purge-kept): {} (system file cache eviction is not performed or verified)",
            cache.label
        );
    }
    let started = Instant::now();
    source.in_scope(|| catalogue::catalogue(&source, &options))?;
    let first = ms(started);
    drop(source);
    let after = kept_files()?;
    println!(
        "first catalogue (kept files {} before, {} after): {first:.1} ms",
        before,
        after.len()
    );
    for file in &after {
        println!(
            "kept file {}: {} bytes",
            file.display(),
            std::fs::metadata(file)?.len()
        );
    }

    println!("sample\tcontent\tbuilt\tbuilt whole\tcontent\tlocalisation\tcatalogue\tloaded whole");
    let (mut built_whole, mut loaded_whole) = (Vec::new(), Vec::new());
    for sample in 0..samples {
        let (source, _) = open()?;
        let (content, built) = source.in_scope(|| {
            let started = Instant::now();
            source.content(&options.options)?;
            let content = ms(started);
            let started = Instant::now();
            catalogue::built_catalogue(&source, &options)?;
            Ok::<_, io::Error>((content, ms(started)))
        })?;
        drop(source);
        let (source, _) = open()?;
        if purge_kept {
            for file in kept_files()? {
                replay_tools::cold_cache::probe_file(
                    &file,
                    replay_tools::cold_cache::ResetScope::FreshSourcePerSample,
                )?;
            }
        }
        let (mount, localisation, loaded) = source.in_scope(|| {
            let started = Instant::now();
            source.content(&options.options)?;
            let mount = ms(started);
            let started = Instant::now();
            dashboard_base::content::localisation(&source, &options)?;
            let localisation = ms(started);
            let started = Instant::now();
            catalogue::catalogue(&source, &options)?;
            Ok::<_, io::Error>((mount, localisation, ms(started)))
        })?;
        drop(source);
        built_whole.push(content + built);
        loaded_whole.push(mount + localisation + loaded);
        println!(
            "{sample}\t{content:.1}\t{built:.1}\t{:.1}\t{mount:.1}\t{localisation:.1}\t{loaded:.1}\t{:.1}",
            content + built,
            mount + localisation + loaded
        );
    }
    for (name, mut wholes) in [("built", built_whole), ("loaded", loaded_whole)] {
        wholes.sort_by(f64::total_cmp);
        if let (Some(least), Some(most)) = (wholes.first(), wholes.last()) {
            println!(
                "{name} ({}): p50 {:.1} ms, least {least:.1}, most {most:.1}",
                cache.label,
                wholes[wholes.len() / 2]
            );
        }
    }
    Ok(())
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1e3
}
