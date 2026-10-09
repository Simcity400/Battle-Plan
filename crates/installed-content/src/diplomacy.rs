//! Diplomatic display keys whose recorded identity settles the native branch.
//! No generic token-to-uppercase construction stands in for a virtual getter.

/// A direct diplomatic-action payment with a nonzero evaluated cost.
/// The action's lexer token is stored at +8, not a database ordinal.
/// Other action kinds need an unrecorded flag or an unported name getter.
pub fn charged_action_name_key(token: u32) -> Option<&'static str> {
    match token {
        // CImproveRelationAction 1410E57B0 stores 0x2CD7. Its cost getter
        // 141111F90 starts at zero and assigns only when +71 is nonzero;
        // GetActionNameKey 1411190E0 then selects data 14294AA70.
        0x2CD7 => Some("DIPLOMACY_IMPROVERELATION"),
        _ => None,
    }
}

/// The daily relation row's own title (140D159A0 -> 140D159B0).
pub fn improve_relation_title_key() -> &'static str {
    "DIPLOMACY_IMPROVERELATION_TITLE"
}

/// Scripted diplomatic actions uppercase their stored template name;
/// GetActionNameKey 140AA3E20 calls SsoStringToUpperInPlace 1424B87F0.
pub fn scripted_action_name_key(template: &str) -> Result<String, String> {
    if !template.is_ascii() {
        return Err(format!(
            "scripted diplomatic action {template}: native locale-dependent uppercase is unavailable (count: 1)"
        ));
    }
    // 1424B87F0 stops conversion at NUL, leaving any stored suffix intact.
    let mut key = template.as_bytes().to_vec();
    for byte in &mut key {
        if *byte == 0 {
            break;
        }
        *byte = byte.to_ascii_uppercase();
    }
    Ok(String::from_utf8(key).expect("ASCII uppercase keeps UTF-8"))
}
