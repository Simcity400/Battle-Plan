use super::*;
use replay_data::rows::{CountryLinkRow, TableId, TableRows};
use replay_frame::frame::TableView;
use viewer_foundation::tooltip_text::NoIcons;

/// Raw 10 and its alias 20 name slot 1 (D01, a dynamic country named after
/// AAA); raw 30 names slot 3 (BBB).
fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 1,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not recorded here".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    put(
        &mut frame,
        TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 10,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 20,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 30,
                canonical_slot: 3,
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::Countries(vec![
            CountryRow {
                id: 10,
                canonical_slot: 1,
                tag: b"D01".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 30,
                canonical_slot: 3,
                tag: b"BBB".to_vec(),
                ..Default::default()
            },
        ]),
    );
    put(
        &mut frame,
        TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 10,
                name_tag: b"AAA".to_vec(),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 30,
                ..Default::default()
            },
        ]),
    );
    frame
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn variant(origin_link: u32, creator_link: u32) -> VariantDefinitionRow {
    VariantDefinitionRow {
        id: 7,
        origin_link,
        creator_link,
        ..Default::default()
    }
}

#[test]
fn an_origin_takes_its_own_tag_and_a_creator_its_original_tag() {
    let frame = frame();
    let tags = Tags::new(&frame);
    // A positive origin, raw or aliased, takes its own tag, never the original.
    assert_eq!(tags.naming_tag(&variant(10, 30)), Ok("D01".into()));
    assert_eq!(tags.naming_tag(&variant(20, 30)), Ok("D01".into()));
    // Without one the creator, raw or aliased, takes its original tag when it has one.
    assert_eq!(tags.naming_tag(&variant(0, 20)), Ok("AAA".into()));
    assert_eq!(tags.naming_tag(&variant(0, 10)), Ok("AAA".into()));
    assert_eq!(tags.naming_tag(&variant(0, 30)), Ok("BBB".into()));
    // A negative signed origin is no origin.
    assert_eq!(
        tags.naming_tag(&variant(0x8000_0000 | 30, 20)),
        Ok("AAA".into())
    );
    assert_eq!(tags.naming_tag(&variant(0, 0)), Ok("---".into()));
    let reason = tags.naming_tag(&variant(0, 40)).unwrap_err();
    assert!(
        reason.contains("country 40 has no CountryLinks row"),
        "{reason}"
    );
}

#[test]
fn only_the_creator_fallback_reads_country_appearance() {
    let mut frame = frame();
    *frame.tables[TableId::CountryAppearance.index()] = TableView::Unavailable {
        reason: "unread".into(),
    };
    let tags = Tags::new(&frame);
    assert_eq!(tags.naming_tag(&variant(20, 30)), Ok("D01".into()));
    let reason = tags.naming_tag(&variant(0, 20)).unwrap_err();
    assert!(reason.contains("CountryAppearance"), "{reason}");
}

#[test]
fn stored_names_are_exact_only_in_the_recording_language() {
    let mut loc = Localisation::default();
    loc.language = "english".into();
    let row = VariantDefinitionRow {
        id: 7,
        display_name: b"Escort".to_vec(),
        variant_name: b"Mk II".to_vec(),
        append_variant_name: 1,
        ..Default::default()
    };
    assert_eq!(
        name(&row, &NoIcons, &loc, None, Some("english")),
        Ok("Escort Mk II".into())
    );
    let reason = name(&row, &NoIcons, &loc, None, Some("german")).unwrap_err();
    assert!(
        reason.contains("variant 7 explicit name is text the game stored in german"),
        "{reason}"
    );
    let reason = name(&row, &NoIcons, &loc, None, None).unwrap_err();
    assert!(reason.contains("has no recording language"), "{reason}");
    // The suffix is read as stored text too.
    let broken = VariantDefinitionRow {
        id: 7,
        display_name: b"Escort".to_vec(),
        variant_name: vec![0xff],
        append_variant_name: 1,
        ..Default::default()
    };
    assert_eq!(
        name(&broken, &NoIcons, &loc, None, Some("english")),
        Err("variant 7 suffix is not UTF-8".into())
    );
}
