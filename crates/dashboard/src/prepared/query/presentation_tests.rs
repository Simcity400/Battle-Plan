//! Stored artwork is named on every prepared reply served while visual mods are shown, whether
//! or not they were shown when the data was prepared.

use super::*;

/// Content options of a request admitted under `presentation`.
fn admitted(presentation: crate::presentation::Presentation) -> ContentOptions {
    ContentOptions {
        options: installed_content_core::ResolveOptions {
            game_dir: "game".into(),
            user_dir: "user".into(),
            cache_dir: "cache".into(),
        },
        allow_unverified: false,
        presentation: crate::presentation::Admitted::new(presentation),
    }
}

/// Data prepared without visual mods, served once some are selected, carries the note in its
/// first content warning list; a reply of a family that stores no artwork, or one served without
/// visual mods or without content, carries none.
#[test]
fn stored_artwork_is_named_once_visual_mods_are_selected() {
    let answered = std::collections::BTreeSet::new();
    let product = "production-lines";
    let plain = admitted(Default::default());
    assert!(served_fallbacks(Some(&plain), product, product, &answered).is_empty());
    assert!(served_fallbacks(None, product, product, &answered).is_empty());
    let styled = admitted(crate::presentation::Presentation {
        visual_mods: vec![installed_content_core::content::visual::Choice {
            id: "mod/art.mod".into(),
            name: "Art".into(),
        }],
        revision: "1".into(),
    });
    let fallbacks = served_fallbacks(Some(&styled), product, product, &answered);
    assert_eq!(
        fallbacks,
        std::collections::BTreeSet::from([
            "production line icon choices (prepared data keeps the recording's artwork)"
                .to_owned()
        ])
    );
    let mut data = serde_json::json!({ "warnings": ["prepared"], "states": [] });
    let line = installed_content_core::content::overlay_refusal_line(&fallbacks);
    append_lines(product, product, line.into_iter().collect(), &mut data);
    let warnings = data["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2);
    assert!(
        warnings[1]
            .as_str()
            .unwrap()
            .starts_with("Artwork from visual mods not shown: 1"),
        "{warnings:?}"
    );
    assert!(served_fallbacks(Some(&styled), "army-size", "army-size", &answered).is_empty());
    // A family whose stored art is drawn again from the selection names none.
    for redrawn in [
        "occupied-states-manifest",
        "trade-flows",
        "ships-sunk",
        "unit-leaders",
        "raids",
        "special-projects",
        "event-feed",
        "operatives-roster",
        "politics-ledger",
    ] {
        assert!(served_fallbacks(Some(&styled), redrawn, redrawn, &answered).is_empty());
    }
}
