//! Acquired crews, read under the crew writers' quiet token: equal tokens on both sides of the
//! read prove no observed crew writer ran across it. `None` is a crew not recorded this hour,
//! never a crew the game holds as none; the rows write it as the column's refusal (`None`).
use engine_native::{Memory, cair_wing, cship, read};

/// A ship's acquired and required crew.
pub(crate) fn ship(
    memory: &impl Memory,
    ship: usize,
    quiet: impl Fn() -> Option<(u64, u64)>,
) -> Option<[i32; 2]> {
    certified(quiet, || {
        let bytes = read::<[u8; 8]>(memory, ship.checked_add(cship::ACQUIRED_MANPOWER)?)?;
        let [a, b, c, d, e, f, g, h] = bytes;
        Some([i32::from_le_bytes([a, b, c, d]), i32::from_le_bytes([e, f, g, h])])
    })
}

/// A wing's acquired manpower and government-in-exile country.
pub(crate) fn wing(
    memory: &impl Memory,
    wing: usize,
    quiet: impl Fn() -> Option<(u64, u64)>,
) -> Option<[i32; 2]> {
    certified(quiet, || {
        Some([
            read::<i32>(memory, wing.checked_add(cair_wing::ACQUIRED_MANPOWER)?)?,
            read::<i32>(memory, wing.checked_add(cair_wing::EXILE_COUNTRY)?)?,
        ])
    })
}

fn certified(
    quiet: impl Fn() -> Option<(u64, u64)>,
    acquire: impl FnOnce() -> Option<[i32; 2]>,
) -> Option<[i32; 2]> {
    let token = quiet()?;
    let value = acquire()?;
    (quiet() == Some(token)).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::watch_registry::WatchRegistry;
    use engine_native::memory::SliceMemory;

    const SHIP: usize = 0x1000;
    const WING: usize = 0x4000;

    fn object(fields: &[(usize, i32)]) -> Vec<u8> {
        let mut bytes = vec![0; 0x1000];
        for &(offset, value) in fields {
            bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    /// The attach hour of every recording, vanilla or modded: both crew writer families are
    /// observed, but the crew registry has no table yet, so a per-receiver certificate refuses
    /// every ship and wing. The game holds the crew.
    #[test]
    fn crews_read_in_an_observed_hour_before_any_registry_table() {
        let registry = WatchRegistry::new();
        registry.set_enabled(true);
        assert_eq!(registry.sequence(SHIP), None, "no table certifies a receiver");
        let ship = object(&[(cship::ACQUIRED_MANPOWER, 255), (cship::ACQUIRED_MANPOWER + 4, 255)]);
        let memory = SliceMemory { base: SHIP, bytes: &ship };
        assert_eq!(super::ship(&memory, SHIP, || registry.quiet()), Some([255, 255]));
        let wing = object(&[(cair_wing::ACQUIRED_MANPOWER, 480), (cair_wing::EXILE_COUNTRY, -1)]);
        let memory = SliceMemory { base: WING, bytes: &wing };
        assert_eq!(super::wing(&memory, WING, || registry.quiet()), Some([480, -1]));
    }

    #[test]
    fn a_crew_is_not_recorded_while_writers_are_unobserved_or_cross_the_read() {
        let ship = object(&[(cship::ACQUIRED_MANPOWER, -1), (cship::ACQUIRED_MANPOWER + 4, i32::MAX)]);
        let memory = SliceMemory { base: SHIP, bytes: &ship };
        let unobserved = WatchRegistry::new();
        assert_eq!(super::ship(&memory, SHIP, || unobserved.quiet()), None);

        let registry = WatchRegistry::new();
        registry.set_enabled(true);
        {
            let _running = registry.notify(SHIP);
            assert_eq!(super::ship(&memory, SHIP, || registry.quiet()), None);
        }
        assert_eq!(
            super::ship(&memory, SHIP, || registry.quiet()),
            Some([-1, i32::MAX]),
            "a returned writer leaves the next read certified"
        );

        let calls = std::cell::Cell::new(0);
        let crossed = || {
            calls.set(calls.get() + 1);
            if calls.get() == 2 {
                drop(registry.notify(0));
            }
            registry.quiet()
        };
        assert_eq!(super::ship(&memory, SHIP, crossed), None, "a writer between the tokens");

        let unreadable = SliceMemory { base: SHIP, bytes: &ship[..cship::ACQUIRED_MANPOWER + 4] };
        assert_eq!(super::ship(&unreadable, SHIP, || registry.quiet()), None);
    }
}
