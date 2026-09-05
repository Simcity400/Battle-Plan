use serde_json::Value;
use crate::world::{Leaf, SubjectAutonomyCurrentRow};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamingRefusal { Missing, Malformed, Identity, DomainGap, Unreadable, InvalidIdentity, InvalidVector, Changed, ClockChanged, InvalidUtf8, OutOfBounds, AutonomyUnbound }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamingInput<T> { Unavailable(NamingRefusal), Absent, Measured(T) }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameTagSource { Base, Override }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveNameTag { pub source: NameTagSource, pub selected_id: i32, pub tag: String }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectNameSemantics {
    pub overlord_tag_db_id: u32,
    pub current_state_hash: NamingInput<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryNameSemantics {
    pub current_leader_subideology: NamingInput<String>,
    pub effective_name_tag: NamingInput<EffectiveNameTag>,
    pub cosmetic_name_base: NamingInput<String>,
    pub ruling_party_group: NamingInput<String>,
    pub subject: NamingInput<SubjectNameSemantics>,
}

impl CountryNameSemantics {
    pub(crate) fn unavailable(reason: NamingRefusal) -> Self {
        Self { current_leader_subideology: NamingInput::Unavailable(reason), effective_name_tag: NamingInput::Unavailable(reason), cosmetic_name_base: NamingInput::Unavailable(reason), ruling_party_group: NamingInput::Unavailable(reason), subject: NamingInput::Unavailable(reason) }
    }
}

fn refusal(value: Option<&Value>) -> NamingRefusal {
    match value.and_then(Value::as_str) {
        Some("unreadable") => NamingRefusal::Unreadable,
        Some("invalid_identity") => NamingRefusal::InvalidIdentity,
        Some("invalid_vector") => NamingRefusal::InvalidVector,
        Some("changed") => NamingRefusal::Changed,
        Some("clock_changed") => NamingRefusal::ClockChanged,
        Some("invalid_utf8") => NamingRefusal::InvalidUtf8,
        Some("out_of_bounds") => NamingRefusal::OutOfBounds,
        _ => NamingRefusal::Malformed,
    }
}

fn token(value: Option<&Value>) -> NamingInput<String> {
    let Some(value) = value else { return NamingInput::Unavailable(NamingRefusal::Missing); };
    let Some(object) = value.as_object() else { return NamingInput::Unavailable(NamingRefusal::Malformed); };
    match object.get("status").and_then(Value::as_str) {
        Some("unavailable") if object.len() == 2 => NamingInput::Unavailable(refusal(object.get("reason"))),
        Some("absent") if object.len() == 1 => NamingInput::Absent,
        Some("measured") if object.len() == 2 => match object.get("value").and_then(Value::as_str) {
            Some(value) if value.len() <= 4096 => NamingInput::Measured(value.to_owned()),
            _ => NamingInput::Unavailable(NamingRefusal::Malformed),
        },
        _ => NamingInput::Unavailable(NamingRefusal::Malformed),
    }
}

fn effective(value: Option<&Value>, country_id: u32, country_tag: &str) -> NamingInput<EffectiveNameTag> {
    let Some(value) = value else { return NamingInput::Unavailable(NamingRefusal::Missing); };
    let parse = || -> Option<EffectiveNameTag> {
        let object = value.as_object()?;
        if object.len() != 4 || value.get("status")?.as_str()? != "measured" { return None; }
        let source = match value.get("source")?.as_str()? { "base" => NameTagSource::Base, "override" => NameTagSource::Override, _ => return None };
        let selected_id = i32::try_from(value.get("selected_id")?.as_i64()?).ok()?;
        if selected_id < 0 || (source == NameTagSource::Override && selected_id == 0) { return None; }
        let tag = value.get("value")?.as_str()?;
        if tag.len() > 4096 { return None; }
        Some(EffectiveNameTag { source, selected_id, tag: tag.to_owned() })
    };
    if value.get("status").and_then(Value::as_str) == Some("unavailable") {
        return match token(Some(value)) { NamingInput::Unavailable(reason) => NamingInput::Unavailable(reason), _ => NamingInput::Unavailable(NamingRefusal::Malformed) };
    }
    match parse() {
        Some(value) if value.source == NameTagSource::Base
            && (value.selected_id as u32 != country_id || value.tag != country_tag) =>
                NamingInput::Unavailable(NamingRefusal::Identity),
        Some(value) => NamingInput::Measured(value),
        None => NamingInput::Unavailable(NamingRefusal::Malformed),
    }
}

pub(crate) fn build(country: Option<&Value>, id: u32, tag: &str, ruling: Option<&Leaf<String>>, autonomy: &Leaf<Vec<SubjectAutonomyCurrentRow>>, gaps: [bool;3]) -> CountryNameSemantics {
    let Some(country) = country else { return CountryNameSemantics::unavailable(NamingRefusal::Missing); };
    let identity = country.get("name_identity");
    let cosmetic = token(identity.and_then(|v| v.get("cosmetic_name_base")));
    let cosmetic = match cosmetic { NamingInput::Absent => NamingInput::Measured(String::new()), other => other };
    let ruling = match ruling { Some(Leaf::Measured(value)) => NamingInput::Measured(value.clone()), _ => NamingInput::Unavailable(NamingRefusal::Missing) };
    let mut result = CountryNameSemantics {
        current_leader_subideology: token(country.get("politics").and_then(|v| v.get("current_leader_subideology"))),
        effective_name_tag: effective(identity.and_then(|v| v.get("effective_name_tag")), id, tag),
        cosmetic_name_base: cosmetic,
        ruling_party_group: ruling,
        subject: subject(country.get("name_subject"), id, autonomy),
    };
    if gaps[0] { result.current_leader_subideology = NamingInput::Unavailable(NamingRefusal::DomainGap); result.ruling_party_group = NamingInput::Unavailable(NamingRefusal::DomainGap); }
    if gaps[1] { result.effective_name_tag = NamingInput::Unavailable(NamingRefusal::DomainGap); result.cosmetic_name_base = NamingInput::Unavailable(NamingRefusal::DomainGap); }
    if gaps[2] { result.subject = NamingInput::Unavailable(NamingRefusal::DomainGap); }
    result
}

fn subject(value: Option<&Value>, id: u32, autonomy: &Leaf<Vec<SubjectAutonomyCurrentRow>>) -> NamingInput<SubjectNameSemantics> {
    let Some(value) = value else { return NamingInput::Unavailable(NamingRefusal::Missing); };
    if value.get("status").and_then(Value::as_str) == Some("unavailable") { return match token(Some(value)) { NamingInput::Unavailable(reason) => NamingInput::Unavailable(reason), _ => NamingInput::Unavailable(NamingRefusal::Malformed) }; }
    let parse = || -> Option<(bool,i32)> {
        if value.as_object()?.len() != 3 || value.get("status")?.as_str()? != "measured" { return None; }
        Some((value.get("has_subject")?.as_bool()?, i32::try_from(value.get("overlord_id")?.as_i64()?).ok()?))
    };
    let Some((present, overlord)) = parse() else { return NamingInput::Unavailable(NamingRefusal::Malformed); };
    if !present || overlord <= 0 { return NamingInput::Absent; }
    let overlord_tag_db_id = overlord as u32;
    if overlord_tag_db_id == id { return NamingInput::Unavailable(NamingRefusal::Identity); }
    let current_state_hash = match autonomy {
        Leaf::Measured(rows) => {
            let mut matches = rows.iter().filter(|row| row.subject_tag_db_id == id);
            match (matches.next(), matches.next()) {
                (Some(row), None) if row.overlord_tag_db_id == overlord_tag_db_id => NamingInput::Measured(row.current_state_hash),
                _ => NamingInput::Unavailable(NamingRefusal::AutonomyUnbound),
            }
        }
        _ => NamingInput::Unavailable(NamingRefusal::AutonomyUnbound),
    };
    NamingInput::Measured(SubjectNameSemantics { overlord_tag_db_id, current_state_hash })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn absence_empty_and_unavailable_remain_distinct() {
        assert_eq!(token(Some(&json!({"status":"absent"}))), NamingInput::Absent);
        assert_eq!(token(Some(&json!({"status":"measured","value":""}))), NamingInput::Measured(String::new()));
        assert_eq!(token(None), NamingInput::Unavailable(NamingRefusal::Missing));
        assert_eq!(token(Some(&json!({"status":"unavailable","reason":"changed"}))), NamingInput::Unavailable(NamingRefusal::Changed));
        assert_eq!(subject(None, 3, &Leaf::Absent), NamingInput::Unavailable(NamingRefusal::Missing));
        assert_eq!(subject(Some(&json!({"status":"measured","has_subject":false,"overlord_id":0})), 3, &Leaf::Absent), NamingInput::Absent);
    }

    #[test]
    fn subject_engine_no_suffix_branch_and_unbound_autonomy_are_distinct() {
        for (present, overlord) in [(false,9), (true,0), (true,-1)] {
            assert_eq!(subject(Some(&json!({"status":"measured","has_subject":present,"overlord_id":overlord})), 3, &Leaf::Absent), NamingInput::Absent);
        }
        assert_eq!(subject(Some(&json!({"status":"measured","has_subject":true,"overlord_id":9})), 3, &Leaf::Absent),
            NamingInput::Measured(SubjectNameSemantics { overlord_tag_db_id:9, current_state_hash:NamingInput::Unavailable(NamingRefusal::AutonomyUnbound) }));
        assert_eq!(subject(Some(&json!({"status":"unavailable","reason":"unreadable"})), 3, &Leaf::Absent), NamingInput::Unavailable(NamingRefusal::Unreadable));
        assert_eq!(subject(Some(&json!({"status":"unavailable","reason":"unreadable","has_subject":false})), 3, &Leaf::Absent), NamingInput::Unavailable(NamingRefusal::Malformed));
    }

    #[test]
    fn effective_tag_preserves_source_and_empty_bytes() {
        assert_eq!(effective(Some(&json!({"status":"measured","source":"override","selected_id":9,"value":""})), 47, "GER"), NamingInput::Measured(EffectiveNameTag { source:NameTagSource::Override, selected_id:9, tag:String::new() }));
        assert!(matches!(effective(Some(&json!({"status":"measured","source":"override","selected_id":0,"value":"GER"})), 47, "GER"), NamingInput::Unavailable(_)));
    }
}
