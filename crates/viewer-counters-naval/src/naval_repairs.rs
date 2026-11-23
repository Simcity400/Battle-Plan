use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;

pub struct Sources<'a>(replay_derive::naval_repairs::Sources<'a>);
impl<'a> Sources<'a> {
    pub fn new(
        frame: &'a DecodedFrame,
        links: &'a BTreeMap<u32, u32>,
        relations: Result<&'a viewer_foundation::visibility::Relations, &'static str>,
    ) -> Self {
        Self(replay_derive::naval_repairs::Sources::new(
            frame, links, relations,
        ))
    }
    pub fn capacity(
        &self,
        base: &replay_data::rows::NavalBaseRow,
    ) -> Result<Capacity, &'static str> {
        self.0.capacity(base).map(|capacity| Capacity {
            used: capacity.used,
            level: capacity.level,
            colour: capacity.colour,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    pub used: i32,
    pub level: i32,
    pub colour: Option<char>,
}
impl Capacity {
    pub fn text(self) -> String {
        let text = format!("{}/{}", self.used, self.level);
        match self.colour {
            Some(colour) => {
                let marker = viewer_native_gui::native_text::COLOR;
                format!("{marker}{colour}{text}{marker}!")
            }
            None => text,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Capacity;
    #[test]
    fn repair_capacity_text_preserves_native_controls() {
        assert_eq!(
            Capacity {
                used: 1,
                level: 10,
                colour: Some('R'),
            }
            .text(),
            format!(
                "{}R1/10{}!",
                viewer_native_gui::native_text::COLOR,
                viewer_native_gui::native_text::COLOR
            )
        );
    }
}
