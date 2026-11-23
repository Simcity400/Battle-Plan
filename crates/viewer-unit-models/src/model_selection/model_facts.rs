use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Fact {
    pub(crate) equipment: usize,
    pub(crate) archetype: usize,
    pub(crate) visual_level: i32,
    pub(crate) model_priority: i32,
    pub(crate) depth: i32,
    pub(crate) has_enabled_module_slots: bool,
}

type Held = HashMap<String, Fact>;

#[derive(Clone, Default)]
pub(crate) struct Retained(Arc<Mutex<BTreeMap<u64, Arc<Held>>>>);

impl fmt::Debug for Retained {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RetainedModelFacts")
    }
}

impl Retained {
    pub(crate) fn census(&self, module_categories: u64) -> View<'_> {
        let held = self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&module_categories)
            .cloned();
        View {
            bank: self,
            module_categories,
            held,
            reached: HashMap::new(),
        }
    }
}

pub(crate) struct View<'a> {
    bank: &'a Retained,
    module_categories: u64,
    held: Option<Arc<Held>>,
    reached: HashMap<&'a str, Fact>,
}

impl<'a> View<'a> {
    pub(crate) fn module_categories(&self) -> u64 {
        self.module_categories
    }

    pub(crate) fn get(&self, key: &str) -> Option<Fact> {
        self.reached
            .get(key)
            .or_else(|| self.held.as_ref()?.get(key))
            .copied()
    }

    pub(crate) fn insert(&mut self, key: &'a str, fact: Fact) {
        self.reached.insert(key, fact);
    }

    pub(crate) fn commit(self) {
        let Self {
            bank,
            module_categories,
            held,
            reached,
        } = self;
        if reached.is_empty() {
            return;
        }
        drop(held);
        let mut banks = bank.0.lock().unwrap_or_else(|p| p.into_inner());
        let held = Arc::make_mut(banks.entry(module_categories).or_default());
        for (key, fact) in reached {
            held.entry(key.to_owned()).or_insert(fact);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn census_snapshots_publish_successes_without_losing_other_views_or_masks() {
        let fact = Fact {
            equipment: 3,
            archetype: 1,
            visual_level: 2,
            model_priority: -7,
            depth: 1,
            has_enabled_module_slots: true,
        };
        let bank = Retained::default();
        let clone = bank.clone();
        let mut first = bank.census(4);
        let mut second = clone.census(4);
        let mut other_mask = clone.census(0);
        first.insert("first", fact);
        assert_eq!(first.get("first"), Some(fact));
        assert_eq!(bank.census(4).get("first"), None);
        first.commit();
        assert_eq!(second.get("first"), None);
        let snapshot = bank.census(4);
        assert_eq!(snapshot.get("first"), Some(fact));
        second.insert("second", fact);
        second.commit();
        assert_eq!(snapshot.get("second"), None);
        let latest = clone.census(4);
        assert_eq!(latest.module_categories(), 4);
        assert_eq!(latest.get("first"), Some(fact));
        assert_eq!(latest.get("second"), Some(fact));
        assert_eq!(other_mask.get("first"), None);
        let disabled = Fact {
            has_enabled_module_slots: false,
            ..fact
        };
        other_mask.insert("first", disabled);
        other_mask.commit();
        assert_eq!(bank.census(0).get("first"), Some(disabled));
        assert_eq!(bank.census(4).get("first"), Some(fact));
        let mut abandoned = bank.census(4);
        abandoned.insert("abandoned", fact);
        drop(abandoned);
        assert_eq!(bank.census(4).get("abandoned"), None);
        assert_eq!(Retained::default().census(4).get("first"), None);
    }
}
