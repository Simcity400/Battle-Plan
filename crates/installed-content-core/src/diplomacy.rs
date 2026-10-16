//! Diplomatic display keys selected by recorded action identity.

/// A direct diplomatic-action payment with a nonzero evaluated cost.
/// The action identity is its lexer token rather than a database ordinal.
/// Other action kinds need an unrecorded flag or an unported name getter.
pub fn charged_action_name_key(token: u32) -> Option<&'static str> {
    match token {
        // The relation action starts with zero cost; recorded nonzero-cost
        // state selects the corresponding action-name key.
        0x2CD7 => Some("DIPLOMACY_IMPROVERELATION"),
        _ => None,
    }
}

/// The daily relation row's own title.
pub fn improve_relation_title_key() -> &'static str {
    "DIPLOMACY_IMPROVERELATION_TITLE"
}

/// Scripted diplomatic actions uppercase their stored template names.
pub fn scripted_action_name_key(template: &str) -> Result<String, String> {
    if !template.is_ascii() {
        return Err(format!(
            "scripted diplomatic action {template}: native locale-dependent uppercase is unavailable (count: 1)"
        ));
    }
    // Uppercasing stops at NUL and leaves any stored suffix intact.
    let mut key = template.as_bytes().to_vec();
    for byte in &mut key {
        if *byte == 0 {
            break;
        }
        *byte = byte.to_ascii_uppercase();
    }
    Ok(String::from_utf8(key).expect("ASCII uppercase keeps UTF-8"))
}
