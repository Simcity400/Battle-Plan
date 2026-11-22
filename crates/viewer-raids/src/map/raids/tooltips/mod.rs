
use viewer_native_gui::lazy_icons::LazyFlags;
mod chance;
pub mod content;
pub(crate) mod intel;
pub mod lane;
mod names;
mod recorded;
pub(super) mod requirements;
mod rules;
mod success;

use super::offers::TooltipTarget;
pub use recorded::{build, details};
#[cfg(any(test, feature = "test-support"))]
pub use recorded::{Inputs, build_from};
use std::sync::Arc;
use viewer_foundation::tooltip_text::Tooltip;

pub struct Content {
    pub assets: Arc<super::RaidAssets>,
    pub flags: Arc<LazyFlags>,
    pub localisation: Arc<content_reader_localisation::localisation::Localisation>,
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
    Type(Arc<TooltipTarget>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Details {
    pub header: Result<String, String>,
    pub entries: Vec<DetailEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DetailEntry {
    pub scope: replay_data::rows::RaidScope,
    pub name: Result<String, String>,
    pub icons: Result<Vec<(usize, Arc<str>, bool)>, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub origin: [f32; 2],
    pub size: [f32; 2],
    pub key: Key,
    pub province: u32,
    pub layer: u64,
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
                province: 0,
                layer: 0,
                supported: true,
            },
            Hit {
                origin: [3., 4.],
                size: [8., 9.],
                key: key(2),
                province: 0,
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
