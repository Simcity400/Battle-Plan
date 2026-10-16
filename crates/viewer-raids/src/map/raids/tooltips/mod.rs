//! Ordinary-map tooltips for own active/result raids, incoming raids and
//! target offers in mode 0. The planning-only unit icon is outside this module.

use viewer_foundation::map::lazy_icons::LazyFlags;
mod chance;
pub mod content;
mod intel;
pub mod lane;
mod names;
mod recorded;
pub(super) mod requirements;
mod rules;
mod success;

use super::offers::TooltipTarget;
pub use recorded::build;
#[cfg(any(test, feature = "test-support"))]
pub use recorded::{Inputs, build_from};
use std::sync::Arc;
use viewer_foundation::tooltip_text::Tooltip;

/// Immutable inputs retained by raid hover requests. Its Arc changes with host content.
pub struct Content {
    pub assets: Arc<super::RaidAssets>,
    pub flags: Arc<LazyFlags>,
    pub localisation: Arc<installed_content::localisation::Localisation>,
    pub recorded_language: String,
    pub scripted: Arc<Result<dashboard_text::scripted_text::ScriptedContent, String>>,
}

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
}
