//! Deferred fold/projection support for compact domain 96 (`PTW1`).
//!
//! Kept independent of the collector producer: `world_fold` owns record
//! ordering and status handling, while this module owns strict decode, the
//! same-boundary D92 identity join, and pointer-free JSON projection.

use std::collections::BTreeMap;

use compact_contract::preferred_tactic::PreferredTacticRow;
use compact_contract::tactic_weight::{
    self as weight96, CountryPreferredTacticWeightRow, CountryPreferredTacticWeightSnapshot,
};
use serde_json::{json, Value};

#[derive(Default)]
pub(crate) struct TacticWeightFoldState {
    seen_this_hour: bool,
    current: Option<CountryPreferredTacticWeightSnapshot>,
}

impl TacticWeightFoldState {
    pub(crate) fn begin_hour(&mut self) {
        self.seen_this_hour = false;
        self.current = None;
    }

    pub(crate) fn mark_unavailable(&mut self) {
        self.seen_this_hour = true;
        self.current = None;
    }

    pub(crate) fn fold_payload(
        &mut self,
        payload: &[u8],
        preferred_rows: &BTreeMap<u32, PreferredTacticRow>,
        preferred_source_sequence: u64,
    ) -> Result<u32, String> {
        self.seen_this_hour = true;
        let decoded = weight96::decode_payload(payload)
            .map_err(|error| format!("country preferred tactic weight PTW1 invalid: {error}"))?;
        if compact_contract::compact_status_is_failure(decoded.status_flags) {
            self.current = None;
            return Ok(decoded.status_flags);
        }
        // `observed_country_count` witnesses the D92 roster the producer walked,
        // and the producer proves `count == preferred.rows.len()` before it
        // encodes (`compact_contract/src/tactic_weight.rs:355`). D18 is a
        // different, deliberately filtered enumeration (`attach_log.rs:30886`)
        // and must never stand in for this count: on `runs/batch3/deepwar_c`
        // PTW1 carries 439 while D18 publishes 356. D18's own relationship to
        // D92 is enforced upstream, by D92's containment gate.
        if decoded.observed_country_count as usize != preferred_rows.len()
            || decoded.source_sequence != preferred_source_sequence
        {
            return Err("PTW1/D92 roster count or source-sequence mismatch".into());
        }
        let actual = decoded
            .rows
            .iter()
            .map(|row| (row.tag_db_id, row))
            .collect::<BTreeMap<_, _>>();
        for (&tag_db_id, selected) in preferred_rows {
            match (
                selected.tactic_database_id,
                selected.tactic_id.as_deref(),
                actual.get(&tag_db_id).copied(),
            ) {
                (0, None, None) => {}
                (id, Some(token), Some(weight))
                    if id == weight.tactic_database_id && token == weight.tactic_id.as_str() => {}
                _ => return Err("PTW1/D92 selected tactic identity mismatch".into()),
            }
        }
        self.current = Some(decoded);
        Ok(self
            .current
            .as_ref()
            .expect("PTW1 snapshot just stored")
            .status_flags)
    }

    pub(crate) fn projected_value(
        &self,
        selected: &PreferredTacticRow,
    ) -> Result<Option<Value>, String> {
        if !self.seen_this_hour {
            return Ok(None);
        }
        let Some(snapshot) = self.current.as_ref() else {
            return Ok(Some(Value::Null));
        };
        let exact = snapshot
            .rows
            .binary_search_by_key(&selected.tag_db_id, |row| row.tag_db_id)
            .ok()
            .map(|index| &snapshot.rows[index]);
        match (
            selected.tactic_database_id,
            selected.tactic_id.as_deref(),
            exact,
        ) {
            (0, None, None) => Ok(Some(Value::Null)),
            (id, Some(token), Some(row))
                if id == row.tactic_database_id && token == row.tactic_id.as_str() =>
            {
                Ok(Some(row_json(snapshot, row)?))
            }
            _ => Err("PTW1/D92 projected identity mismatch".into()),
        }
    }
}

fn row_json(
    snapshot: &CountryPreferredTacticWeightSnapshot,
    row: &CountryPreferredTacticWeightRow,
) -> Result<Value, String> {
    Ok(json!({
        "authority": "a729_country_preferred_tactic_weight_v1",
        "carrier": "PTW1",
        "domain_id": weight96::DOMAIN_ID,
        "executable_sha256": "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad",
        "consumer_evidence_sha256": "ffa7e5b14d1e26ed2a16fc669a3a7293ec4a01cd4a2216f9aa90ff91f3749d73",
        "define_evidence_sha256": "3c7e36d639beba725d948523e4c2a3256ae0c596cc2e38b66755f7ab2b2f88cd",
        "lookup_evidence_sha256": "0422c6cd871adedbbaedce5363e3163bbe03dd2f8ed0d79879f4bb77e71acef8",
        "runtime_observation_sha256": "91a5f2b75e86f3a7b1c299a1fa07a8fef95202b30370a3a1421ca643649f4a00",
        "tactic_key_census_sha256": "9203ff38de7c3d7f29043d2d428fea44d607c73133e9956f317da3b8c4a47da1",
        "nonzero_override_observation_sha256": "f4d71c3e5a4c73bb1a52887679c8ceccbcd60c872729d604bc10a0ea55d3073c",
        "fixed_scale": weight96::FIXED_SCALE,
        "source_sequence": snapshot.source_sequence,
        "observed_country_count": snapshot.observed_country_count,
        "tag_db_id": row.tag_db_id,
        "tactic_database_id": row.tactic_database_id,
        "tactic_id": row.tactic_id,
        "modifier_key_runtime_provenance": row.modifier_key,
        "modifier_metadata_flags": row.modifier_metadata_flags,
        "modifier_metadata_gate": row.modifier_metadata_gate,
        "accumulator_entry_present": row.accumulator_entry_present,
        "country_factor_raw": snapshot.country_factor_raw,
        "accumulator_raw": row.accumulator_raw,
        "effective_bonus_raw": row.effective_bonus_raw,
        "multiplier_numerator_raw": row.multiplier_numerator_raw()?,
    }))
}
