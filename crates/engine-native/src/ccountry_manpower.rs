//! Country-embedded manpower operands. Mobilization is independent of the free
//! manpower pool; these layouts do not supply a new manpower walker.

use engine_sites::Rva;
/// Residual, i32, added to the controlled-state pool sum.
pub const RESIDUAL: usize = 0xc;
/// Country-tag handle, u32.
pub const COUNTRY: usize = 8;
/// Signed i64 mobilization ratio.
pub const MOBILIZATION: usize = 0x10;
/// Daily mobilization and exile growth using the embedded object.
pub const DAILY_UPDATE_RVA: Rva = Rva(0x00d5_d890);

use crate::{Memory, ccountry, read};

/// Exile pool and its stored admission cap, signed people.
pub const EXILE_POOL: usize = 0x18;
/// Diplomacy's signed host reference; positive means the country is in exile.
pub const HOST: usize = 0x1f0;
pub const LEGITIMACY: usize = 0x1f8;

/// Daily-sliced operands. A missing scalar is its own refusal. Exile-only
/// resources are absent for a known non-exile, without touching their memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub ratio: Option<i64>,
    pub host: Option<i32>,
    pub legitimacy: Option<i64>,
    pub leaders: Option<i32>,
    pub exile_pool: Option<i32>,
    pub exile_cap: Option<i32>,
}

/// The caller already authenticated both country and diplomacy, including
/// its owner backlink. Called only for the existing daily slice and writers'
/// marked countries; this is not a census and allocates nothing.
pub fn read_authenticated(memory: &impl Memory, country: usize, diplomacy: usize) -> Row {
    let mut row = Row {
        ratio: country
            .checked_add(ccountry::MANPOWER + MOBILIZATION)
            .and_then(|at| read(memory, at)),
        host: diplomacy.checked_add(HOST).and_then(|at| read(memory, at)),
        ..Row::default()
    };
    if row.host.is_some_and(|host| host > 0) {
        if let Some(words) = country
            .checked_add(ccountry::MANPOWER + EXILE_POOL)
            .and_then(|at| read::<[u8; 8]>(memory, at))
        {
            row.exile_pool = Some(i32::from_le_bytes(words[..4].try_into().unwrap()));
            row.exile_cap = Some(i32::from_le_bytes(words[4..].try_into().unwrap()));
        }
        if let Some(words) = diplomacy
            .checked_add(LEGITIMACY)
            .and_then(|at| read::<[u8; 12]>(memory, at))
        {
            row.legitimacy = Some(i64::from_le_bytes(words[..8].try_into().unwrap()));
            row.leaders = Some(i32::from_le_bytes(words[8..].try_into().unwrap()));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fixture {
        host: i32,
        reads: Cell<usize>,
        fault: Option<usize>,
    }
    impl Memory for Fixture {
        fn copy(&self, at: usize, out: &mut [u8]) -> bool {
            self.reads.set(self.reads.get() + 1);
            if self.fault == Some(at) {
                return false;
            }
            let value = match at {
                x if x == 0x1000 + ccountry::MANPOWER + MOBILIZATION => {
                    (-17_i64).to_le_bytes().to_vec()
                }
                x if x == 0x5000 + HOST => self.host.to_le_bytes().to_vec(),
                x if x == 0x1000 + ccountry::MANPOWER + EXILE_POOL && self.host > 0 => {
                    [(-9_i32).to_le_bytes(), 8123_i32.to_le_bytes()].concat()
                }
                x if x == 0x5000 + LEGITIMACY && self.host > 0 => [
                    1234567_i64.to_le_bytes().as_slice(),
                    (-3_i32).to_le_bytes().as_slice(),
                ]
                .concat(),
                _ => panic!("unexpected native read {at:x}"),
            };
            out.copy_from_slice(&value);
            true
        }
    }
    #[test]
    fn manpower_exile_demand_and_independent_refusals() {
        for index in 0..6 {
            let host = if index % 3 == 0 { 73 } else { -1 };
            let fixture = Fixture {
                host,
                reads: Cell::new(0),
                fault: None,
            };
            let row = read_authenticated(&fixture, 0x1000, 0x5000);
            assert_eq!((row.ratio, row.host), (Some(-17), Some(host)));
            assert_eq!(fixture.reads.get(), if host > 0 { 4 } else { 2 });
            if host > 0 {
                assert_eq!(
                    (row.exile_pool, row.exile_cap, row.legitimacy, row.leaders),
                    (Some(-9), Some(8123), Some(1234567), Some(-3))
                );
            } else {
                assert_eq!(row.exile_pool, None);
                assert_eq!(row.legitimacy, None);
            }
        }
        let fixture = Fixture {
            host: 73,
            reads: Cell::new(0),
            fault: Some(0x1000 + ccountry::MANPOWER + EXILE_POOL),
        };
        let row = read_authenticated(&fixture, 0x1000, 0x5000);
        assert_eq!(row.exile_pool, None);
        assert_eq!(row.exile_cap, None);
        assert_eq!(row.legitimacy, Some(1234567));
        assert_eq!(fixture.reads.get(), 4);
    }
}
