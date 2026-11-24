use dashboard_base::{Source, SourceKey};
use dashboard_core::config::Config;
use dashboard_text::metadata::link_slot;
use replay::campaign::SeekResult;
use replay_data::rows::{TableId, tables};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::AtomicBool,
    time::Instant,
};
use viewer::game_windows::{GameWindowRequest, Service};

fn decode_base64(input: &str) -> Vec<u8> {
    let val = |b: u8| -> u8 {
        match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => 0,
        }
    };
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            break;
        }
        let a = val(chunk[0]) as u32;
        let b = val(chunk[1]) as u32;
        let c = val(chunk[2]) as u32;
        let d = val(chunk[3]) as u32;
        let n = (a << 18) | (b << 12) | (c << 6) | d;
        out.push(((n >> 16) & 0xff) as u8);
        if chunk[2] != b'=' {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if chunk[3] != b'=' {
            out.push((n & 0xff) as u8);
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: equipment_parity_dump <replay.battleplan> <game_dir> <user_dir> <out_dir>");
        std::process::exit(2);
    }
    let replay_path = PathBuf::from(&args[1]);
    let game_dir = PathBuf::from(&args[2]);
    let user_dir = PathBuf::from(&args[3]);
    let out_dir = PathBuf::from(&args[4]);
    fs::create_dir_all(&out_dir)?;

    let source = Source::open(
        SourceKey {
            source_id: "parity-test".to_owned(),
            revision: "rev-1".to_owned(),
        },
        replay_path,
        &AtomicBool::new(false),
    )?;
    let replay = source.replay_ref();
    let timelines = replay.timelines().ok_or("replay has no timelines")?;
    let branch_id = timelines.main;
    let first_hour = timelines
        .list
        .iter()
        .find(|t| t.id == branch_id)
        .map(|t| t.first_hour())
        .ok_or("main branch not found")?;
    let hour = first_hour;
    println!("branch={branch_id} first_hour={first_hour}");

    let tables_needed = [
        TableId::VariantDefinitions,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::SimulationFeatures,
    ];
    let frame = match replay.seek_selected(branch_id, hour, Some(&tables_needed))? {
        SeekResult::Frame(frame) => frame,
        SeekResult::Gap { reason, .. } => return Err(format!("gap: {reason}").into()),
    };
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let lexical = frame.complete::<tables::LexicalTokens>()?;
    let defs = frame.complete::<tables::VariantDefinitions>()?;
    let dlc_mask = frame
        .complete::<tables::SimulationFeatures>()
        .ok()
        .and_then(|rows| rows.first())
        .map_or(0, |r| r.dlc_mask);
    println!("dlc_mask=0x{dlc_mask:x} total_variants={}", defs.len());

    let ger_slot = countries
        .iter()
        .find(|c| content_reader_parser::parser::c_string(&c.tag) == b"GER")
        .map(|c| c.canonical_slot)
        .ok_or("GER not found")?;
    let spell = |tok: u32| -> String {
        lexical
            .binary_search_by_key(&tok, |r| r.token)
            .ok()
            .and_then(|i| {
                std::str::from_utf8(content_reader_parser::parser::c_string(&lexical[i].literal))
                    .ok()
                    .map(str::to_owned)
            })
            .unwrap_or_default()
    };

    let mut targets: Vec<(&str, u64, String)> = Vec::new();
    let wanted = [
        ("ship", "Zerstörer 1934A", "ship_hull_light_2"),
        ("tank", "Panzer II", "light_tank_chassis_1"),
        ("plane", "Ju 87", "small_plane_cas_airframe_1"),
        ("upgrade", "Sdkfz 222", "armored_car_equipment_1"),
    ];
    let ger_defs: Vec<_> = defs
        .iter()
        .filter(|def| link_slot(links, def.creator_link) == Some(ger_slot))
        .collect();
    for (kind, needle_name, needle_eq) in wanted {
        let matched = ger_defs
            .iter()
            .find(|def| {
                let name = String::from_utf8_lossy(content_reader_parser::parser::c_string(
                    &def.display_name,
                ));
                name.contains(needle_name)
            })
            .or_else(|| {
                ger_defs
                    .iter()
                    .find(|def| spell(def.equipment_token) == needle_eq)
            })
            .or_else(|| {
                (kind == "upgrade")
                    .then(|| {
                        ger_defs.iter().find(|def| {
                            let eq = spell(def.equipment_token);
                            eq.contains("armored_car") || eq.contains("mechanized") || eq.contains("motorized")
                        })
                    })
                    .flatten()
            });
        if let Some(def) = matched {
            let name = String::from_utf8_lossy(content_reader_parser::parser::c_string(
                &def.display_name,
            ))
            .into_owned();
            let suffix = String::from_utf8_lossy(content_reader_parser::parser::c_string(
                &def.variant_name,
            ))
            .into_owned();
            let eq = spell(def.equipment_token);
            println!(
                "matched {kind}: id={} eq={eq} name={name:?} suffix={suffix:?} append={} icon={:?}",
                def.id,
                def.append_variant_name,
                String::from_utf8_lossy(content_reader_parser::parser::c_string(&def.icon))
            );
            targets.push((kind, def.id, format!("{name} {suffix}")));
        }
    }

    let config = Config {
        game_dir,
        user_dir,
        older_game_dirs: Vec::new(),
    };
    let service = Service::default();

    for (kind, variant_id, label) in &targets {
        let req = GameWindowRequest {
            source_id: "parity-test".to_owned(),
            revision: "rev-1".to_owned(),
            branch_id: branch_id.to_string(),
            hour,
            kind: "equipmentDesigner".to_owned(),
            target: serde_json::json!({ "tag": "GER", "variant": variant_id.to_string() }),
            scale: 1.0,
            scroll: BTreeMap::new(),
            sub_view: None,
        };
        let t0 = Instant::now();
        let rendered = source.in_scope(|| service.render_from_source(&source, &config, &[], req.clone()))?;
        let cold_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let t1 = Instant::now();
        let _ = source.in_scope(|| service.render_from_source(&source, &config, &[], req))?;
        let warm_ms = t1.elapsed().as_secs_f64() * 1000.0;

        let kind_dir = out_dir.join(kind);
        fs::create_dir_all(&kind_dir)?;
        let png_bytes = decode_base64(&rendered.png_base64);
        fs::write(kind_dir.join("rendered.png"), &png_bytes)?;
        println!(
            "rendered {kind} ({label}): size={:?} px={:?} regions={} close={} cold={cold_ms:.1}ms warm={warm_ms:.1}ms warnings={:?}",
            rendered.logical_size,
            rendered.pixel_size,
            rendered.regions.len(),
            rendered.regions.iter().filter(|r| r.close).count(),
            rendered.warnings
        );
    }
    Ok(())
}
