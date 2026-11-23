use super::*;
use replay_data::rows::{ShipCounterInputs, ShipModelRow};
use std::cell::Cell;

fn row() -> ShipModelRow {
    ShipModelRow {
        variant: 7,
        formation_flags: 1 << 8,
        fallback_sprite: b"shape\0ignored".to_vec(),
        counter: Some(ShipCounterInputs {
            definition_valid: 1,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn generated_keys_keep_full_handles_flags_raw_definition_and_bytes() {
    let baseline = row();
    let key = Key::new(&baseline, Some((100, 1)));
    let mut variants = Vec::new();
    let mut changed = baseline.clone();
    changed.variant |= 1 << 40;
    variants.push((changed, Some((100, 1))));
    let mut changed = baseline.clone();
    changed.formation_flags |= 1 << 63;
    variants.push((changed, Some((100, 1))));
    let mut changed = baseline.clone();
    changed.counter.as_mut().unwrap().definition_valid = 2;
    variants.push((changed, Some((100, 1))));
    let mut changed = baseline.clone();
    changed.fallback_sprite.push(255);
    variants.push((changed, Some((100, 1))));
    variants.push((baseline.clone(), Some((200, 1))));
    variants.push((baseline.clone(), Some((100, 2))));
    variants.push((baseline.clone(), None));
    let mut facts = Facts::default();
    facts.resolve(key, || Ok(Some("base".into()))).unwrap();
    for (row, definition) in &variants {
        assert!(Key::new(row, *definition) != key);
        assert_eq!(
            facts
                .resolve(Key::new(row, *definition), || Ok(Some("distinct".into())))
                .unwrap()
                .as_deref(),
            Some("distinct")
        );
    }
    assert_eq!(facts.0.len(), variants.len() + 1);
}

#[test]
fn only_successful_generated_values_are_published_including_known_none() {
    let row = row();
    let key = Key::new(&row, Some((100, 1)));
    let mut facts = Facts::default();
    for reason in ["first", "changed refusal", "late conventional refusal"] {
        assert_eq!(
            facts.resolve(key, || Err(reason.into())),
            Err(reason.into())
        );
        assert!(facts.0.is_empty());
    }
    let calls = Cell::new(0);
    for _ in 0..4 {
        assert_eq!(
            facts
                .resolve(key, || {
                    calls.set(calls.get() + 1);
                    Ok(None)
                })
                .unwrap(),
            None
        );
    }
    assert_eq!(calls.get(), 1);
    let mut next_pass = Facts::default();
    assert_eq!(
        next_pass
            .resolve(key, || Ok(Some("different world/content/limits".into())))
            .unwrap()
            .as_deref(),
        Some("different world/content/limits")
    );
}

#[test]
fn successful_none_keeps_each_occurrences_fallback_tag_and_refusal_live() {
    let row = row();
    let key = Key::new(&row, Some((100, 1)));
    let mut facts = Facts::default();
    let generated_calls = Cell::new(0);
    for (tag, expected) in [
        (Ok("AAA"), Ok(Some("AAA_shape_0_entity".to_owned()))),
        (
            Err("country unavailable".to_owned()),
            Err("country unavailable".to_owned()),
        ),
        (Ok("BBB"), Ok(Some("BBB_shape_0_entity".to_owned()))),
    ] {
        let actual = choose(
            &row,
            b"",
            || tag,
            || {
                facts.resolve(key, || {
                    generated_calls.set(generated_calls.get() + 1);
                    Ok(None)
                })
            },
            |name| Ok(name.starts_with("AAA_") || name.starts_with("BBB_")),
        );
        assert_eq!(actual, expected);
    }
    assert_eq!(generated_calls.get(), 1);
}

#[test]
fn pass_keeps_explicit_and_invalid_definition_demands_outside_generation() {
    let frame = equipment_graphics::test_support::empty_frame();
    let mut assets = UnitModelAssets::off(String::new());
    assets
        .registered
        .extend(["saved".into(), PLACEHOLDER_ENTITY.into()]);
    let row = row();
    let mut invalid = row.clone();
    invalid.counter.as_mut().unwrap().definition_valid = 0;
    let mut pass = Pass::new(&assets, &frame, None);
    assert_eq!(
        pass.name(|| panic!("unreached tag"), &row, b"saved\0\xff", None)
            .unwrap()
            .as_deref(),
        Some("saved")
    );
    assert!(pass.facts.0.is_empty());
    for _ in 0..3 {
        assert_eq!(
            pass.name(
                || panic!("tag after failed generation"),
                &row,
                b"",
                Some((100, 1))
            ),
            Err("naval_entity: hourly graphics inputs are unavailable".into())
        );
        assert!(
            pass.facts.0.is_empty(),
            "actual generated errors must not publish"
        );
    }
    assert_eq!(
        pass.name(|| panic!("invalid definition tag"), &invalid, b"\xff", None)
            .unwrap()
            .as_deref(),
        Some(PLACEHOLDER_ENTITY)
    );
    assert!(pass.facts.0.is_empty());
}

#[test]
fn cached_generated_success_does_not_cache_registration_failure_or_change_demand() {
    let row = row();
    let key = Key::new(&row, Some((100, 1)));
    let mut facts = Facts::default();
    let generated_calls = Cell::new(0);
    let registration_calls = Cell::new(0);
    for _ in 0..4 {
        let result = choose(
            &row,
            b"",
            || panic!("winner refusal must not fall back"),
            || {
                facts.resolve(key, || {
                    generated_calls.set(generated_calls.get() + 1);
                    Ok(Some("winner".into()))
                })
            },
            |name| {
                assert_eq!(name, "winner");
                registration_calls.set(registration_calls.get() + 1);
                Err("declaration refused".into())
            },
        );
        assert_eq!(result, Err("declaration refused".into()));
    }
    assert_eq!(generated_calls.get(), 1);
    assert_eq!(registration_calls.get(), 4);
}

#[test]
fn warm_pass_hits_skip_generation_and_a_new_pass_cannot_see_them() {
    let frame = equipment_graphics::test_support::empty_frame();
    let mut assets = UnitModelAssets::off(String::new());
    assets.registered.insert("winner".into());
    let row = row();
    let definition = Some((100, 1));
    let mut pass = Pass::new(&assets, &frame, None);
    pass.facts
        .resolve(Key::new(&row, definition), || Ok(Some("winner".into())))
        .unwrap();
    for _ in 0..4 {
        assert_eq!(
            pass.name(
                || panic!("winner must not read fallback country"),
                &row,
                b"",
                definition
            ),
            Ok(Some("winner".into()))
        );
    }
    let mut next_pass = Pass::new(&assets, &frame, None);
    assert_eq!(
        next_pass.name(
            || panic!("failed generation must not read fallback country"),
            &row,
            b"",
            definition
        ),
        Err("naval_entity: hourly graphics inputs are unavailable".into())
    );
    assert!(next_pass.facts.0.is_empty());
}
