//! Ordinary-map tooltips for own active/result raids, incoming raids and
//! target offers in mode 0. The planning-only unit icon is outside this module.

mod chance;
pub(super) mod content;
mod intel;
pub mod lane;
mod names;
mod recorded;
pub(super) mod requirements;
mod rules;
mod success;

use super::offers::TooltipTarget;
use crate::combat::text::ensure_newlines;
pub use recorded::build;
#[cfg(test)]
pub(crate) use recorded::{Inputs, build_from};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Raid {
        id: u32,
        reference_type: u32,
        incoming: bool,
    },
    Offer(Arc<[TooltipTarget]>),
}

/// Interface pixels in window paint order; reverse iteration selects the
/// topmost admitted icon.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub key: Key,
    pub layer: u64,
    /// Unsupported native hit shapes block lower windows without dispatching observers.
    pub supported: bool,
}
impl Hit {
    pub fn contains(&self, point: [f32; 2]) -> bool {
        (0..2).all(|i| point[i] >= self.origin[i] && point[i] < self.origin[i] + self.size[i])
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tooltip {
    pub title: String,
    pub body: String,
    /// Line name and why its input/text cannot be reproduced; no guessed
    /// text stands in for it. Counts are occurrences, not unique reasons.
    pub refused: BTreeMap<String, usize>,
}

impl Tooltip {
    fn refuse(&mut self, line: &str, reason: String) {
        *self.refused.entry(format!("{line}: {reason}")).or_default() += 1;
    }
    fn line(&mut self, body: bool, gap: usize, name: &str, value: Result<String, String>) {
        match value {
            Ok(value) => {
                let out = if body {
                    &mut self.body
                } else {
                    &mut self.title
                };
                if !value.is_empty() {
                    ensure_newlines(out, gap);
                    out.push_str(&value);
                }
            }
            Err(reason) => self.refuse(name, reason),
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod basics {
    use super::*;

    #[test]
    fn hit_rectangles_use_draw_order_and_exclude_the_far_edges() {
        let key = |id| Key::Raid {
            id,
            reference_type: 4,
            incoming: false,
        };
        let hits = [
            Hit {
                origin: [1., 2.],
                size: [8., 9.],
                key: key(1),
                layer: 0,
                supported: true,
            },
            Hit {
                origin: [3., 4.],
                size: [8., 9.],
                key: key(2),
                layer: 0,
                supported: true,
            },
        ];
        assert_eq!(
            hits.iter()
                .rev()
                .find(|h| h.contains([4., 5.]))
                .unwrap()
                .key,
            key(2)
        );
        assert!(!hits[0].contains([9., 5.]));
        assert!(!hits[0].contains([5., 11.]));
    }

    #[test]
    fn refused_lines_are_named_and_counted_without_hiding_other_lines() {
        let mut tooltip = Tooltip::default();
        tooltip.line(false, 2, "one", Ok("first\n".into()));
        for _ in 0..2 {
            tooltip.line(false, 2, "two", Err("absent operand".into()));
        }
        tooltip.line(false, 2, "three", Ok("last".into()));
        assert_eq!(tooltip.title, "first\n\nlast");
        assert_eq!(tooltip.refused["two: absent operand"], 2);
    }
}
