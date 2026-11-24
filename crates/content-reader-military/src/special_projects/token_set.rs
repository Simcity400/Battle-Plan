
#[derive(Clone, Copy, Default)]
struct Slot {
    token: u32,
    distance: u8,
}

#[derive(Default)]
struct Set {
    slots: Vec<Slot>,
    count: usize,
    mask: usize,
    maximum: u8,
}

fn hash(mut token: u32) -> u32 {
    token = (token ^ (token >> 16)).wrapping_mul(0x045d_9f3b);
    token = (token ^ (token >> 16)).wrapping_mul(0x045d_9f3b);
    token ^ (token >> 16)
}

impl Set {
    fn grow(&mut self) {
        let buckets = if self.mask <= 3 {
            8
        } else {
            (self.mask + 1) * 2
        };
        let old = std::mem::replace(
            &mut self.slots,
            vec![Slot::default(); buckets + buckets.trailing_zeros() as usize + 3],
        );
        self.mask = buckets - 1;
        self.maximum = buckets.trailing_zeros() as u8 + 2;
        self.count = 0;
        for slot in old.into_iter().filter(|slot| slot.distance != 0) {
            self.insert(slot.token);
        }
    }

    fn insert(&mut self, token: u32) {
        if self.slots.is_empty() {
            self.slots = vec![Slot::default()];
        }
        let mut position = hash(token) as usize & self.mask;
        let mut distance = 1;
        while self.slots[position].distance != 0 {
            if self.slots[position].token == token {
                return;
            }
            position += 1;
            distance += 1;
            if distance > self.slots[position].distance {
                break;
            }
        }
        if distance > self.maximum || (self.count + 1) as f32 / self.mask as f32 > 0.9_f32 {
            self.grow();
            self.insert(token);
            return;
        }
        if self.slots[position].distance == 0 {
            self.slots[position] = Slot { token, distance };
            self.count += 1;
            return;
        }
        let mut carried = std::mem::replace(&mut self.slots[position], Slot { token, distance });
        carried.distance += 1;
        let mut at = position + 1;
        loop {
            if self.slots[at].distance == 0 {
                self.slots[at] = carried;
                self.count += 1;
                return;
            }
            if carried.distance > self.slots[at].distance {
                std::mem::swap(&mut carried, &mut self.slots[at]);
            }
            carried.distance += 1;
            if carried.distance > self.maximum {
                std::mem::swap(&mut carried, &mut self.slots[position]);
                self.grow();
                self.insert(carried.token);
                return;
            }
            at += 1;
        }
    }
}

pub fn order(tokens: &[u32]) -> Vec<u32> {
    let mut set = Set::default();
    for token in tokens {
        set.insert(*token);
    }
    set.slots
        .into_iter()
        .filter(|slot| slot.distance != 0)
        .map(|slot| slot.token)
        .collect()
}

