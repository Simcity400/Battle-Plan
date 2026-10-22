//! What a served prepared reply names as drawn from the recording.

use super::*;

/// An answer's own fallbacks are named in the reply's first content warning list; an answer
/// that drew none adds no line.
#[test]
fn an_answers_fallbacks_are_named_in_its_first_content_warning_list() {
    let mut data = serde_json::json!({ "warnings": ["prepared"], "states": [] });
    let product = "occupied-states-manifest";
    let none = installed_content_core::content::overlay_refusal_line(&Default::default());
    append_lines(product, product, none.into_iter().collect(), &mut data);
    assert_eq!(data["warnings"].as_array().unwrap().len(), 1);
    let answered = std::collections::BTreeSet::from(["occupation law icon (fixture)".to_owned()]);
    let line = installed_content_core::content::overlay_refusal_line(&answered);
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
}
