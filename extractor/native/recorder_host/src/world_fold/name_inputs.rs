use serde_json::{json, Value};
use compact_contract::country_name_inputs::{NameToken, EffectiveNameTag, NameInputRefusal, NameTagSource};

pub(super) fn token_value(token: &NameToken) -> Value {
    match token {
        NameToken::Unavailable(reason) => json!({"status":"unavailable", "reason":reason.name()}),
        NameToken::Absent => json!({"status":"absent"}),
        NameToken::Present(value) => json!({"status":"measured", "value":value}),
    }
}

pub(super) fn effective_value(value: &Result<EffectiveNameTag, NameInputRefusal>) -> Value {
    match value {
        Err(reason) => json!({"status":"unavailable", "reason":reason.name()}),
        Ok(value) => json!({"status":"measured", "value":value.tag,
            "source":match value.source { NameTagSource::Base => "base", NameTagSource::Override => "override" },
            "selected_id":value.selected_id}),
    }
}
