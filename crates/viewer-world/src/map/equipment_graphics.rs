//! Authored equipment graphic pools, shared with the dashboard through
//! [`installed_content_equipment_graphics::equipment_graphics`]; the viewer's model bridge and legacy ladders
//! stay here. The pool rules and native evidence live in that shared module.
pub use installed_content_equipment_graphics::equipment_graphics::*;
use std::cell::RefCell;
pub mod bridge;
pub mod context;
pub mod legacy;

use installed_content_core::parser::Item;
use std::{collections::BTreeMap, sync::Arc};

/// Per-hour memo view; native selects anew on every lookup.
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    evaluate:
        &'a dyn Fn(&[Item], dashboard_text::triggers::Scopes) -> dashboard_text::triggers::Eval,
    values: &'a LimitValues,
}

/// A memo owned by the hour, never by a unit, country or content load.
#[derive(Default)]
pub struct LimitValues(RefCell<BTreeMap<usize, (Arc<[Item]>, dashboard_text::triggers::Eval)>>);

/// The pools an hour decided, their items and what each decided.
pub type Decided = BTreeMap<usize, (Arc<[Item]>, Result<bool, String>)>;

impl<'a> Limits<'a> {
    pub fn new(
        values: &'a LimitValues,
        evaluate: &'a dyn Fn(
            &[Item],
            dashboard_text::triggers::Scopes,
        ) -> dashboard_text::triggers::Eval,
    ) -> Self {
        Self { evaluate, values }
    }

    /// Every pool decided so far this hour.
    pub fn decided(&self) -> Decided {
        self.values
            .0
            .borrow()
            .iter()
            .map(|(&pool, (items, eval))| (pool, (Arc::clone(items), eval.clone().decided())))
            .collect()
    }

    /// Whether every pool of `decided` decides the same this hour.
    pub fn decide_the_same(&self, decided: &Decided) -> bool {
        decided
            .iter()
            .all(|(&pool, (items, was))| self.holds(pool, items) == *was)
    }
}

impl PoolLimits for Limits<'_> {
    fn holds(&self, pool: usize, items: &[Item]) -> Result<bool, String> {
        self.values
            .0
            .borrow_mut()
            .entry(pool)
            .or_insert_with(|| {
                // Each evaluation starts a fresh temporary context even for
                // the empty scope. Kept answers re-decide every pool each hour instead of
                // recording what deciding it read ([`Limits::decide_the_same`]).
                let eval = replay_frame::frame::view_reads::unrecorded(|| {
                    (self.evaluate)(items, dashboard_text::triggers::Scopes::empty())
                });
                (items.into(), eval)
            })
            .1
            .clone()
            .decided()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

#[cfg(test)]
pub mod tests;
