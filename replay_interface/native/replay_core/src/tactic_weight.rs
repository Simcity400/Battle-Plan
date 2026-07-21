//! Typed admission for the domain-96 preferred-tactic weight sidecar.

use serde_json::{Map, Value};

use crate::world::Leaf;

pub const FIXED_SCALE: i64 = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryPreferredTacticWeight {
    pub source_sequence: u64,
    pub observed_country_count: u32,
    pub modifier_key_runtime_provenance: u32,
    pub accumulator_entry_present: bool,
    pub country_factor_raw: i64,
    pub accumulator_raw: i64,
    pub effective_bonus_raw: i64,
    pub multiplier_numerator_raw: i64,
}

pub(crate) fn parse_preferred_weight(
    preferred_sidecar: &Map<String, Value>,
    expected_tag_db_id: u32,
    expected_tactic_database_id: i32,
    expected_tactic_id: Option<&str>,
    ctx: &str,
    warnings: &mut Vec<String>,
) -> Leaf<CountryPreferredTacticWeight> {
    let Some(raw) = preferred_sidecar.get("preferred_weight") else {
        return Leaf::Absent;
    };
    let Some(raw) = raw.as_object() else {
        if !raw.is_null() {
            warnings.push(format!(
                "{ctx}.extra._r2_preferred_tactic_live.preferred_weight: expected object or null"
            ));
        }
        return Leaf::Unobtained;
    };
    let u32_field = |name: &str| {
        raw.get(name)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
    };
    let i32_field = |name: &str| {
        raw.get(name)
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
    };
    let i64_field = |name: &str| raw.get(name).and_then(Value::as_i64);
    let Some(source_sequence) = raw.get("source_sequence").and_then(Value::as_u64) else {
        warnings.push(format!("{ctx}: PTW1 source sequence missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(observed_country_count) = u32_field("observed_country_count") else {
        warnings.push(format!("{ctx}: PTW1 observed roster count missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(modifier_key_runtime_provenance) = u32_field("modifier_key_runtime_provenance") else {
        warnings.push(format!("{ctx}: PTW1 modifier key missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(accumulator_entry_present) = raw
        .get("accumulator_entry_present")
        .and_then(Value::as_bool)
    else {
        warnings.push(format!("{ctx}: PTW1 accumulator presence missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(country_factor_raw) = i64_field("country_factor_raw") else {
        warnings.push(format!("{ctx}: PTW1 country factor missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(accumulator_raw) = i64_field("accumulator_raw") else {
        warnings.push(format!("{ctx}: PTW1 accumulator value missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(effective_bonus_raw) = i64_field("effective_bonus_raw") else {
        warnings.push(format!("{ctx}: PTW1 effective bonus missing/invalid"));
        return Leaf::Unobtained;
    };
    let Some(multiplier_numerator_raw) = i64_field("multiplier_numerator_raw") else {
        warnings.push(format!("{ctx}: PTW1 multiplier numerator missing/invalid"));
        return Leaf::Unobtained;
    };
    let authority_ok = raw.get("authority").and_then(Value::as_str)
        == Some("a729_country_preferred_tactic_weight_v1")
        && raw.get("carrier").and_then(Value::as_str) == Some("PTW1")
        && raw.get("domain_id").and_then(Value::as_u64) == Some(96)
        && raw.get("fixed_scale").and_then(Value::as_i64) == Some(FIXED_SCALE)
        && source_sequence != 0
        && observed_country_count != 0
        && preferred_sidecar
            .get("source_sequence")
            .and_then(Value::as_u64)
            == Some(source_sequence)
        && raw.get("executable_sha256").and_then(Value::as_str)
            == Some("870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad")
        && raw.get("consumer_evidence_sha256").and_then(Value::as_str)
            == Some("ffa7e5b14d1e26ed2a16fc669a3a7293ec4a01cd4a2216f9aa90ff91f3749d73")
        && raw.get("define_evidence_sha256").and_then(Value::as_str)
            == Some("3c7e36d639beba725d948523e4c2a3256ae0c596cc2e38b66755f7ab2b2f88cd")
        && raw.get("lookup_evidence_sha256").and_then(Value::as_str)
            == Some("0422c6cd871adedbbaedce5363e3163bbe03dd2f8ed0d79879f4bb77e71acef8")
        && raw
            .get("runtime_observation_sha256")
            .and_then(Value::as_str)
            == Some("91a5f2b75e86f3a7b1c299a1fa07a8fef95202b30370a3a1421ca643649f4a00")
        && raw.get("tactic_key_census_sha256").and_then(Value::as_str)
            == Some("9203ff38de7c3d7f29043d2d428fea44d607c73133e9956f317da3b8c4a47da1")
        && raw
            .get("nonzero_override_observation_sha256")
            .and_then(Value::as_str)
            == Some("f4d71c3e5a4c73bb1a52887679c8ceccbcd60c872729d604bc10a0ea55d3073c")
        && u32_field("tag_db_id") == Some(expected_tag_db_id)
        && i32_field("tactic_database_id") == Some(expected_tactic_database_id)
        && expected_tactic_id.is_some()
        && raw.get("tactic_id").and_then(Value::as_str) == expected_tactic_id
        && modifier_key_runtime_provenance != 0
        && u32_field("modifier_metadata_flags").is_some_and(|flags| flags & 0x10 == 0)
        && i32_field("modifier_metadata_gate") == Some(0)
        && (accumulator_entry_present || accumulator_raw == 0)
        && country_factor_raw.checked_add(accumulator_raw) == Some(effective_bonus_raw)
        && FIXED_SCALE.checked_add(effective_bonus_raw) == Some(multiplier_numerator_raw);
    if !authority_ok {
        warnings.push(format!(
            "{ctx}.extra._r2_preferred_tactic_live.preferred_weight: authority/identity/arithmetic mismatch"
        ));
        return Leaf::Unobtained;
    }
    Leaf::Measured(CountryPreferredTacticWeight {
        source_sequence,
        observed_country_count,
        modifier_key_runtime_provenance,
        accumulator_entry_present,
        country_factor_raw,
        accumulator_raw,
        effective_bonus_raw,
        multiplier_numerator_raw,
    })
}
