
pub fn charged_action_name_key(token: u32) -> Option<&'static str> {
    match token {
        IMPROVE_RELATIONS => Some("DIPLOMACY_IMPROVERELATION"),
        _ => None,
    }
}

pub fn improve_relation_title_key() -> &'static str {
    "DIPLOMACY_IMPROVERELATION_TITLE"
}

pub fn scripted_action_name_key(template: &str) -> Result<String, String> {
    if !template.is_ascii() {
        return Err(format!(
            "scripted diplomatic action {template}: native locale-dependent uppercase is unavailable (count: 1)"
        ));
    }
    let mut key = template.as_bytes().to_vec();
    for byte in &mut key {
        if *byte == 0 {
            break;
        }
        *byte = byte.to_ascii_uppercase();
    }
    Ok(String::from_utf8(key).expect("ASCII uppercase keeps UTF-8"))
}

const NAVAL_BLOCKADE: u32 = 0x27F8;
const AIR_BASE_ACCESS: u32 = 0x283A;
const OFFER_AIR_BASE_ACCESS: u32 = 0x2855;
const GUARANTEE: u32 = 0x28DA;
const MILITARY_ACCESS: u32 = 0x2934;
const OFFER_MILITARY_ACCESS: u32 = 0x2FC8;
const IMPROVE_RELATIONS: u32 = 0x2CD7;
const NON_AGGRESSION_PACT: u32 = 0x2FBC;
const BOOST_PARTY_POPULARITY: u32 = 0x30ED;
const LEND_LEASE: u32 = 0x314A;
const EMBARGO: u32 = 0x3274;
const EQUIPMENT_PURCHASE: u32 = 0x33E8;
const MARKET_ACCESS_RIGHTS: u32 = 0x3580;
const ATTACHE: u32 = 0x38D9;
const DOCKING_RIGHTS: u32 = 0x3AFA;
const OFFER_DOCKING_RIGHTS: u32 = 0x3AFB;
const LICENCE_PRODUCTION_ACCESS: u32 = 0x4B83;

pub fn relation_title_key(token: u32, spelling: &str) -> Result<&'static str, String> {
    let key = match token {
        NAVAL_BLOCKADE => "DIPLOMACY_NAVAL_BLOCKADE_TITLE",
        AIR_BASE_ACCESS | OFFER_AIR_BASE_ACCESS => "DIPLOMACY_AIR_BASE_ACCESS",
        GUARANTEE => "DIPLOMACY_GUARANTEE_TITLE",
        MILITARY_ACCESS | OFFER_MILITARY_ACCESS => "DIPLOMACY_MILACC",
        IMPROVE_RELATIONS => improve_relation_title_key(),
        NON_AGGRESSION_PACT => "DIPLOMACY_NONAGGRESSIONPACT",
        EMBARGO => "DIPLOMACY_EMBARGO_TITLE",
        MARKET_ACCESS_RIGHTS => "DIPLOMACY_INTERNATIONAL_MARKET_ACCESS_RIGHTS",
        ATTACHE => "DIPLOMACY_SEND_ATTACHE",
        DOCKING_RIGHTS | OFFER_DOCKING_RIGHTS => "DIPLOMACY_DOCKING_RIGHTS",
        LICENCE_PRODUCTION_ACCESS => "DIPLOMACY_LICACC",
        BOOST_PARTY_POPULARITY => {
            return Err(format!(
                "Relation {spelling}: its ideology-group title operand is not recorded (count: 1)"
            ));
        }
        LEND_LEASE | EQUIPMENT_PURCHASE => {
            return Err(format!(
                "Relation {spelling}: its relation-name selector supplies no title; action titles are not relation names (count: 1)"
            ));
        }
        _ => {
            return Err(format!(
                "Relation {spelling}: its in-game title selector is not supported (count: 1)"
            ));
        }
    };
    Ok(key)
}

#[cfg(test)]
mod tests {
    #[test]
    fn relation_titles_refuse_unverified_selectors_without_guessing_from_spelling() {
        assert_eq!(
            super::relation_title_key(super::IMPROVE_RELATIONS, "synthetic relation").unwrap(),
            super::improve_relation_title_key()
        );
        let error = super::relation_title_key(987654, "modded_relation").unwrap_err();
        assert!(error.contains("modded_relation") && error.contains("count: 1"));
        assert!(super::relation_title_key(987654, super::improve_relation_title_key()).is_err());
    }
}
