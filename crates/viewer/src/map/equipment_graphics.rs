//! Authored equipment graphic pools, shared with the dashboard through
//! [`installed_content::equipment_graphics`]; the viewer's model bridge and legacy ladders
//! stay here. The pool rules and native evidence live in that shared module.
pub use installed_content::equipment_graphics::*;
pub mod bridge;
pub mod context;
pub mod legacy;

use installed_content::parser::Item;
use std::collections::BTreeMap;

/// Per-hour memo view; native selects anew on every lookup (1409F1540).
#[derive(Clone, Copy)]
pub struct Limits<'a> {
    evaluate: &'a dyn Fn(&[Item], dashboard::triggers::Scopes) -> dashboard::triggers::Eval,
    values: &'a LimitValues,
}

/// A memo owned by the hour, never by a unit, country or content load.
#[derive(Default)]
pub struct LimitValues(std::cell::RefCell<BTreeMap<usize, dashboard::triggers::Eval>>);

impl<'a> Limits<'a> {
    pub fn new(
        values: &'a LimitValues,
        evaluate: &'a dyn Fn(&[Item], dashboard::triggers::Scopes) -> dashboard::triggers::Eval,
    ) -> Self {
        Self { evaluate, values }
    }
}

impl PoolLimits for Limits<'_> {
    fn holds(&self, pool: usize, items: &[Item]) -> Result<bool, String> {
        self.values
            .0
            .borrow_mut()
            .entry(pool)
            .or_insert_with(|| {
                // CTrigger::Check starts a fresh temp context even for
                // the empty scope (140540570 -> 1405354A0 -> 140544190).
                (self.evaluate)(items, dashboard::triggers::Scopes::empty())
            })
            .clone()
            .decided()
    }
}

#[cfg(test)]
pub(crate) mod tests;
