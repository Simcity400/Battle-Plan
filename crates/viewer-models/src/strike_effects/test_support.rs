use super::*;

pub fn presentation(slots: [Option<usize>; 2]) -> Presentation {
    Presentation::new(Assets {
        entities: [slots[0], slots[1], Some(5), Some(6)],
    })
}
