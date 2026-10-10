//! Exact passive priority stock and first unfiltered convoy variant joins.
use crate::rows::{Recorded, TransportConvoyVariant, TransportConvoys};
pub type Fact<T> = Result<T, &'static str>;

pub fn current(input: &Recorded<Option<TransportConvoys>>, hour: i32) -> Fact<&TransportConvoys> {
    let value = match input {
        Recorded::NotRecorded => return Err("transport convoys not recorded"),
        Recorded::Value(None) => return Err("transport original convoy loan refused"),
        Recorded::Value(Some(value)) => value,
    };
    if value.sample_hour != hour {
        return Err("transport original convoy loan is stale");
    }
    Ok(value)
}

/// Truncate the held/dirty i64 total, then consume low signed32. Each
/// reached priority<=requested reservation subtracts in signed32 with wrap.
pub fn available(value: &TransportConvoys, priority: i32) -> Fact<i32> {
    let cache = value
        .cache
        .ok_or("transport original convoy cache refused")?;
    let total = if cache.valid != 0 {
        cache.raw.ok_or("transport held convoy cache refused")?
    } else {
        let mut total = 0_i64;
        for entry in value
            .pool
            .as_deref()
            .ok_or("transport dirty convoy pool refused")?
        {
            let flags = entry
                .flags
                .ok_or("transport dirty convoy classifier refused")?;
            if flags & 1 != 0 {
                total = total.wrapping_add(entry.quantity).clamp(
                    -engine::cconvoys::POOL_CLAMP_MAGNITUDE,
                    engine::cconvoys::POOL_CLAMP_MAGNITUDE,
                );
            }
        }
        total
    };
    let mut result = (total / 100_000) as i32;
    for reservation in value
        .reservations
        .as_deref()
        .ok_or("transport signed convoy reservations refused")?
    {
        if reservation.priority <= priority {
            result = result.wrapping_sub(reservation.quantity);
        }
    }
    Ok(result)
}

pub fn first(value: &TransportConvoys) -> Fact<Option<&TransportConvoyVariant>> {
    value
        .first_variant
        .as_ref()
        .map(Option::as_ref)
        .ok_or("transport original first nonzero convoy variant refused")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rows::{TransportConvoyCache, TransportConvoyEntry, TransportConvoyReservation};
    #[test]
    fn signed_priority_and_dirty_wrapping_clamp_are_native() {
        let mut value = TransportConvoys {
            cache: Some(TransportConvoyCache {
                valid: 0,
                raw: None,
            }),
            pool: Some(vec![
                TransportConvoyEntry {
                    flags: Some(1),
                    quantity: i64::MAX,
                    ..Default::default()
                },
                TransportConvoyEntry {
                    flags: Some(1),
                    quantity: i64::MAX,
                    ..Default::default()
                },
            ]),
            reservations: Some(vec![
                TransportConvoyReservation {
                    priority: -2,
                    quantity: -3,
                },
                TransportConvoyReservation {
                    priority: 8,
                    quantity: i32::MAX,
                },
            ]),
            ..Default::default()
        };
        let total = engine::cconvoys::POOL_CLAMP_MAGNITUDE
            .wrapping_add(i64::MAX)
            .clamp(
                -engine::cconvoys::POOL_CLAMP_MAGNITUDE,
                engine::cconvoys::POOL_CLAMP_MAGNITUDE,
            );
        assert_eq!(
            available(&value, 0),
            Ok(((total / 100_000) as i32).wrapping_add(3))
        );
        value.cache = Some(TransportConvoyCache {
            valid: 255,
            raw: Some(-150_000),
        });
        value.pool = None;
        assert_eq!(available(&value, 0), Ok(2));
        assert_eq!(available(&value, 8), Ok(2_i32.wrapping_sub(i32::MAX)));
        value.reservations = None;
        assert!(available(&value, 0).is_err());
    }
}
