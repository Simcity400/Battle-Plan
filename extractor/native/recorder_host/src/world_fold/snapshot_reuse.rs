use super::*;
use snapshot::canon_value::{key_cmp, key_cmp_decoded, CanonValue};
use std::sync::Arc;

pub(super) struct RetainedResearch {
    country_path: Vec<String>,
    value: CanonValue,
}

impl WorldFold {
    pub(super) fn invalidate_research_root(&mut self, tag_db_id: u32) {
        let key = tag_db_id.to_string();
        if self.canonical_research.contains_key(&key) {
            self.dirty_canonical_research.insert(key);
        }
    }

    fn prepare_canonical_research(&mut self) {
        self.canonical_research.retain(|key, _| {
            self.countries
                .get(key)
                .and_then(|country| country.get("research"))
                .is_some_and(Value::is_object)
        });
        self.dirty_canonical_research
            .retain(|key| self.canonical_research.contains_key(key));
        for (key, country) in &self.countries {
            let Some(value @ Value::Object(_)) = country.get("research") else {
                continue;
            };
            if self.dirty_canonical_research.remove(key)
                || !self.canonical_research.contains_key(key)
            {
                let canonical = snapshot::replay_cache::cacheprof::timed(
                    &snapshot::replay_cache::cacheprof::PARSE_NS,
                    || {
                        snapshot::canon_convert::from_value_shared(
                            value,
                            self.canonical_research.get(key),
                        )
                    },
                );
                self.canonical_research.insert(key.clone(), canonical);
            }
        }
    }

    pub(super) fn build_reused_snapshot(
        &mut self,
        hour_index: i64,
        date: &str,
    ) -> (Value, Vec<RetainedResearch>) {
        self.prepare_canonical_research();
        let mut roots = Vec::with_capacity(self.canonical_research.len());
        let value = self.build_snapshot_with_research(hour_index, date, Some(&mut roots));
        (value, roots)
    }

    pub(super) fn locate_research_roots(
        &self,
        countries: &Map<String, Value>,
        withheld: &WithheldCountries,
        plan: &CountryKeyPlan,
        roots: &mut Vec<RetainedResearch>,
    ) {
        for (id_text, value) in &self.canonical_research {
            let id: u32 = id_text
                .parse()
                .expect("a retained country has a numeric identity");
            let emitted = plan.emitted_tag.get(&id).cloned().or_else(|| {
                (!self.country_identity_known_this_hour && !self.tag_by_db_id.contains_key(&id))
                    .then(|| id_text.clone())
            });
            let country_path = match emitted.filter(|key| countries.contains_key(key)) {
                Some(key) => vec!["countries".into(), key],
                None if withheld.by_tag_db_id.contains_key(id_text) => vec![
                    "tag_db_id_index".into(),
                    "withheld_countries".into(),
                    "by_tag_db_id".into(),
                    id_text.clone(),
                    "section".into(),
                ],
                None => panic!("a retained research owner must be emitted or accounted for"),
            };
            roots.push(RetainedResearch {
                country_path,
                value: value.clone(),
            });
        }
    }
}

pub(super) fn restore_research_roots(value: &mut CanonValue, roots: Vec<RetainedResearch>) {
    for root in roots {
        let mut country = &mut *value;
        for key in root.country_path {
            let CanonValue::Object(node) = country else {
                unreachable!("a retained research path names country objects");
            };
            let members = Arc::make_mut(node).contents_mut();
            let index = members
                .binary_search_by(|(stored, _)| key_cmp_decoded(stored, &key))
                .expect("a retained research path exists in this snapshot");
            country = &mut members[index].1;
        }
        let CanonValue::Object(node) = country else {
            unreachable!("research belongs to a country object");
        };
        let members = Arc::make_mut(node).contents_mut();
        match members.binary_search_by(|(key, _)| key_cmp_decoded(key, "research")) {
            Ok(index) => {
                let overlay = std::mem::replace(&mut members[index].1, CanonValue::Null);
                members[index].1 = merge_research(root.value, overlay);
            }
            Err(index) => members.insert(index, ("research".into(), root.value)),
        }
    }
}

fn merge_research(base: CanonValue, overlay: CanonValue) -> CanonValue {
    let (CanonValue::Object(base_node), CanonValue::Object(overlay_node)) = (&base, overlay) else {
        unreachable!("D13 research and the D45 overlay are objects");
    };
    if overlay_node.is_empty() {
        return base;
    }
    let mut members = base_node.to_vec();
    for (key, value) in overlay_node.iter() {
        match members.binary_search_by(|(stored, _)| key_cmp(stored, key)) {
            Ok(index) => members[index].1 = value.clone(),
            Err(index) => members.insert(index, (key.clone(), value.clone())),
        }
    }
    CanonValue::object(members)
}
