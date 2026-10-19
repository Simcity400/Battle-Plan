//! Products against recorded inputs: a real recording's product impact, and the
//! rule that products date tables only by the recording's format.

use replay_frame::frame::TableView;
use std::sync::{Arc, atomic::AtomicBool};

#[test]
#[ignore = "read-only G2 recording; BATTLE_PLAN_COMPAT_RECORDINGS supplies its path"]
fn compatibility_real_g2_product_impact() {
    use crate::{
        products::{self, LoadContext, Product},
        scope::{Context, RecordedScopes, Target, Unresolved},
    };
    use installed_content_core::{scope::Keyword, scope::Scope, tag_aliases::TagAliases};
    use installed_content_localisation::localisation::Localisation;
    let path = std::env::var("BATTLE_PLAN_COMPAT_RECORDINGS").or_else(|_| std::env::var("HOI4_COMPAT_RECORDINGS")).unwrap();
    let cancelled = AtomicBool::new(false);
    let source = crate::Source::open(
        crate::SourceKey {
            source_id: "g2".into(),
            revision: "read-only".into(),
        },
        path.into(),
        &cancelled,
    )
    .unwrap();
    let replay = source.replay();
    assert_eq!(replay.format.version(), 94);
    let replay::container::SeekResult::Frame(frame) = replay.seek(2, 60827796).unwrap() else {
        panic!("missing G2 comparison hour")
    };
    let countries = frame
        .complete::<replay_data::rows::tables::Countries>()
        .unwrap();
    let country = countries
        .iter()
        .find(|country| {
            country.canonical_slot > 0
                && country.tag.len() == 3
                && country.tag.iter().all(u8::is_ascii_alphabetic)
        })
        .unwrap();
    let tag = std::str::from_utf8(&country.tag).unwrap();
    let localisation = Localisation::default();
    let styles = products::country_styles::HourStyles::read(&frame, Some(&localisation)).unwrap();
    let coloured = countries
        .iter()
        .filter(|country| {
            let tag = std::str::from_utf8(&country.tag).unwrap();
            styles.get(tag).is_some_and(|style| style.color.is_some())
        })
        .count();
    assert!(coloured > 0);
    let flags = products::country_flags::choices(&frame).unwrap();
    assert!(!flags.is_empty());
    let appearances = frame
        .complete::<replay_data::rows::tables::CountryAppearance>()
        .unwrap();
    let links = frame
        .complete::<replay_data::rows::tables::CountryLinks>()
        .unwrap();
    for flag in &flags {
        let canonical = links
            .iter()
            .find(|row| row.id == flag.raw)
            .unwrap()
            .canonical_slot;
        let country = countries
            .iter()
            .rev()
            .find(|row| row.canonical_slot == canonical)
            .unwrap();
        let Some(appearance) = appearances.iter().find(|row| row.country == country.id) else {
            assert!(
                flag.refusal
                    .as_deref()
                    .is_some_and(|why| why.contains("CountryAppearance"))
            );
            continue;
        };
        // Format 94 retains cosmetic candidates but has no exact signed
        // original-tag selection operand, even for an aliased raw link.
        let refusal =
            installed_content_country_names::country_names::original_tag(country, appearance).unwrap_err();
        assert!(refusal.contains("CountryAppearance.original_tag_reference not recorded"));
        assert!(
            flag.refusal
                .as_deref()
                .is_some_and(|why| why.contains(&refusal))
        );
    }
    let names = installed_content_country_names::country_names::CountryNames::read(&frame, &localisation);
    assert!(names.operands(country.canonical_slot).unwrap().is_ok());
    let scopes = RecordedScopes::read(&frame, Arc::new(TagAliases::default()));
    assert_eq!(scopes.resolve_tag(tag).unwrap(), country.canonical_slot);
    let context = Context::country(country.canonical_slot);
    assert!(matches!(
        scopes.resolve(&Scope::Keyword(Keyword::Overlord), &context),
        Ok(_) | Err(Unresolved::Nothing(_))
    ));
    assert_eq!(
        scopes
            .resolve(&Scope::Keyword(Keyword::Root), &context)
            .unwrap(),
        Target::Country(country.canonical_slot)
    );
    let context = LoadContext { content: None };
    let measure = products::country_measure::CountryMeasure::load(
        &source,
        &products::country_measure::MeasureParams {
            branch_id: "2".into(),
            measure: products::country_measure::Measure::CommandPower,
            tags: vec![tag.into()],
        },
        &context,
        &cancelled,
    )
    .unwrap();
    assert!(measure.series.iter().any(|series| !series.runs.is_empty()));
    let owners = products::land_owners::land_owners(&replay, None, &cancelled).unwrap();
    println!(
        "G2_PRODUCT country-styles: {} colour rows; country naming operands retained (localisation not loaded in this check); country-flags: {} native cosmetic/original choices; OVERLORD retained",
        coloured,
        flags.len()
    );
    println!(
        "G2_PRODUCT command-power: {} runs; land-owners: {} countries",
        measure
            .series
            .iter()
            .map(|series| series.runs.len())
            .sum::<usize>(),
        owners.countries.len()
    );
    for table in [
        replay_data::rows::TableId::CountryAppearance,
        replay_data::rows::TableId::DivisionTemplates,
        replay_data::rows::TableId::Decisions,
        replay_data::rows::TableId::DivisionMovement,
    ] {
        assert!(matches!(
            &*frame.tables[table.index()],
            TableView::Complete(_)
        ));
    }
}

/// Products date tables only through the recording's format
/// (`replay_data::cadence::cadence_in`, through [`crate::metadata::slot_read`]), so a later
/// cadence change cannot redate an older recording. Test modules may name
/// today's cadence to build their fixtures.
#[test]
fn products_never_read_todays_cadence_directly() {
    fn visit(directory: &std::path::Path, offenders: &mut Vec<String>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "tests") {
                    visit(&path, offenders);
                }
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if !name.ends_with(".rs")
                || name.ends_with("tests.rs")
                || matches!(
                    name.as_str(),
                    "metadata.rs" | "test_fixtures.rs" | "mod_fixtures.rs"
                )
            {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // An inline test module ends the production code it follows.
            let production = text
                .lines()
                .take_while(|line| {
                    !(line.starts_with("mod ") && line.contains("tests") && line.ends_with('{'))
                })
                .collect::<Vec<_>>();
            for (index, line) in production.iter().enumerate() {
                if line.contains("cadence::cadence(") || line.contains("cadence::is_refresh(") {
                    offenders.push(format!("{}:{}", path.display(), index + 1));
                }
            }
        }
    }
    let mut offenders = Vec::new();
    // The other dashboard crates (core, text, base and the product crates) hold code that was in
    // this crate; it keeps the rule.
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for entry in std::fs::read_dir(&crates).unwrap() {
        let path = entry.unwrap().path();
        if path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("dashboard"))
        {
            visit(&path.join("src"), &mut offenders);
        }
    }
    assert!(offenders.is_empty(), "{offenders:#?}");
}
