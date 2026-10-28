use super::*;
use std::io::Write;

pub(super) fn run(options: &Options, path: &Path) -> io::Result<bool> {
    let (Some(game), Some(user)) = (&options.game, &options.user) else {
        return Err(invalid(
            "Display products require --game-dir and --user-dir",
        ));
    };
    replay_frame::cpu::enable();
    let started = Instant::now();
    let cpu = replay_frame::cpu::process_cpu_ns();
    engine_executable::executable_tables::initialize(game)?;
    let replay = replay::container::Replay::open_path(&options.recording)?;
    let resolve = installed_content_core::ResolveOptions {
        game_dir: game.clone(),
        user_dir: user.clone(),
        cache_dir: path.join("cache/content"),
    };
    let resolved = installed_content_core::resolve(&replay.identity, &resolve)?;
    let mut warnings = Vec::new();
    dashboard::content::check_replay(&replay, options.allow_unverified, &mut warnings)?;
    resolved.append_changed_to(&mut warnings);
    let language = dashboard::content::language(&resolve)?;
    let pilot =
        dashboard_game_window::display_products::Pilot::load(&resolved.layers, user, &language)?;
    let mut output = BufWriter::new(File::create(path.join("display-products-cost.jsonl"))?);
    writeln!(
        output,
        "{}",
        json!({"measurement":"display-products-content","wallMs":started.elapsed().as_secs_f64()*1000.0,
        "processCpuNs":cpu.zip(replay_frame::cpu::process_cpu_ns()).map(|(a,b)| b.saturating_sub(a)),
        "coldScope":"fresh pilot content session including lazy cells and snow; OS file cache not evicted","allCountries":options.display_products_all_countries,
        "warnings":warnings,"recording":options.recording})
    )?;
    let requested: Requests = match &options.requests {
        Some(file) => serde_json::from_slice(&fs::read(file)?)?,
        None => Requests::default(),
    };
    let selected = dashboard_base::prepared_map::selected_tables();
    let mut blocks = 0u64;
    let mut hours = 0u64;
    for index in &replay.index {
        if requested
            .branch_id
            .as_ref()
            .is_some_and(|b| b != &index.branch.to_string())
        {
            continue;
        }
        if !requested.hours.is_empty()
            && !requested
                .hours
                .iter()
                .any(|h| (index.first_hour..=index.last_hour).contains(h))
        {
            continue;
        }
        let updates = replay.read_block_recorded_selected(index, selected)?;
        blocks += 1;
        let mut decoder = replay_frame::frame::FrameDecoder::for_tables(selected);
        for update in &updates {
            let frame = decoder.advance(update)?;
            if !requested.hours.is_empty() && !requested.hours.contains(&frame.hour) {
                continue;
            }
            hours += 1;
            pilot.observe(
                frame,
                options.display_products_all_countries,
                &path.join("display-snapshots"),
                |row| {
                    writeln!(output, "{row}")?;
                    Ok(())
                },
            )?;
        }
    }
    let witness = pilot.content_witness(&path.join("display-content-witness.bin"))?;
    writeln!(
        output,
        "{}",
        json!({"measurement":"display-products-total","acceptedObservedHours":hours,
        "sourceBlockReadCalls":blocks,"sourceReadScope":"explicit harness selected-block calls; internal anchor/section work in CPU readerOperations",
        "contentWitness":witness,"wallMs":started.elapsed().as_secs_f64()*1000.0,
        "processCpuNs":cpu.zip(replay_frame::cpu::process_cpu_ns()).map(|(a,b)| b.saturating_sub(a)),
        "cpu":replay_frame::cpu::snapshot(),"performanceAcceptance":false})
    )?;
    output.flush()?;
    Ok(true)
}
