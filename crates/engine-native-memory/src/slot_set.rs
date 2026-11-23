
pub const DIRECT: u32 = 1 << 16;
const WORDS: usize = DIRECT as usize / 64;
const REMEMBERED: usize = 128;

pub struct SlotSet {
    bits: [u64; WORDS],
    marked: [u32; REMEMBERED],
    marked_len: usize,
    spilled: bool,
}

impl SlotSet {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            bits: [0; WORDS],
            marked: [0; REMEMBERED],
            marked_len: 0,
            spilled: false,
        }
    }

    pub fn insert<E>(
        &mut self,
        slot: u32,
        seen_before: impl FnOnce() -> Result<bool, E>,
    ) -> Result<bool, E> {
        if slot >= DIRECT {
            return seen_before();
        }
        let (word, bit) = ((slot / 64) as usize, 1u64 << (slot % 64));
        if self.bits[word] & bit != 0 {
            return Ok(true);
        }
        self.bits[word] |= bit;
        if self.marked_len < REMEMBERED {
            self.marked[self.marked_len] = slot;
            self.marked_len += 1;
        } else {
            self.spilled = true;
        }
        Ok(false)
    }

    pub fn clear(&mut self) {
        if self.spilled {
            self.bits = [0; WORDS];
        } else {
            for &slot in &self.marked[..self.marked_len] {
                self.bits[(slot / 64) as usize] = 0;
            }
        }
        self.marked_len = 0;
        self.spilled = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> Result<bool, ()> {
        panic!("direct slots never use the fallback")
    }

    #[test]
    fn detects_repeats_among_many_slots_and_clears_between_owners() {
        let mut set = Box::new(SlotSet::new());
        for slot in (1..=20_000).rev() {
            assert_eq!(set.insert(slot, never), Ok(false));
        }
        for slot in 1..=20_000 {
            assert_eq!(set.insert(slot, never), Ok(true));
        }
        set.clear();
        assert!(set.bits.iter().all(|word| *word == 0));
        for slot in [7, 64, 65_535] {
            assert_eq!(set.insert(slot, never), Ok(false));
        }
        assert_eq!(set.insert(64, never), Ok(true));
        set.clear();
        assert!(set.bits.iter().all(|word| *word == 0));
        assert!(!set.spilled && set.marked_len == 0);
    }

    #[test]
    fn slots_beyond_the_bitmap_use_the_callers_exact_check() {
        let mut set = SlotSet::new();
        assert_eq!(set.insert(DIRECT, || Ok::<_, ()>(false)), Ok(false));
        assert_eq!(set.insert(u32::MAX, || Ok::<_, ()>(true)), Ok(true));
        assert_eq!(set.insert(DIRECT + 1, || Err("fault")), Err("fault"));
    }
}
