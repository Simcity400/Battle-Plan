//! What the engagement folds read of an hour, from any source of its tables
//! ([`Tables`]): a decoded frame, or the fold reader's rows kept from lanes.
//! [`available`] is [`crate::table_access::rows`] over that source; [`viewer`] is
//! [`crate::map::perspective::viewer`] over it.

use crate::{
    map::perspective::{Country, Resolution},
    table_access::Available,
};
use replay_data::rows::{TableSchema, tables};
use replay_frame::frame::{OperandStatus, Rowset, Tables};
use std::{collections::BTreeMap, io};

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn decoded<T: TableSchema, F: Tables + ?Sized>(frame: &F) -> io::Result<&[T::Row]> {
    match frame.rows::<T>(u64::MAX) {
        Some(Rowset::Decoded(rows)) => Ok(rows),
        None => Ok(&[]),
    }
}

/// The rows of `T` this hour holds, whatever state its table is in.
pub fn available<T: TableSchema, F: Tables + ?Sized>(frame: &F) -> io::Result<Available<'_, T>> {
    match frame.table_status(T::ID) {
        OperandStatus::Complete => Ok(Available::Complete(decoded::<T, F>(frame)?)),
        OperandStatus::Truncated(missing) => Ok(Available::Truncated { rows: decoded::<T, F>(frame)?, missing }),
        OperandStatus::Unavailable(reason) => Ok(Available::Unavailable { reason }),
        OperandStatus::NotRecorded(mode) => Ok(Available::NotRecorded { mode }),
        OperandStatus::WrongLayout | OperandStatus::TruncatedWrongLayout(_) => Err(invalid(format!("hour {} {:?} contains another table's rows", frame.at_hour(), T::ID))),
    }
}

/// Every row of `T` when the table is whole; none otherwise.
pub fn whole<T: TableSchema, F: Tables + ?Sized>(frame: &F) -> Option<&[T::Row]> {
    match available::<T, F>(frame) {
        Ok(Available::Complete(rows)) => Some(rows),
        _ => None,
    }
}

/// The client's stored viewpoint, including the native observer fallback
/// ([`crate::map::allegiance::perspective`]).
pub fn perspective<F: Tables + ?Sized>(frame: &F, links: &BTreeMap<u32, u32>) -> Option<u32> {
    let [player] = whole::<tables::LocalPlayer, F>(frame)? else { return None };
    player.viewing_country().and_then(|raw| links.get(&raw).copied())
}

/// The viewing country's slot this hour, and how it was chosen
/// ([`crate::map::perspective::viewer`]), from any source of the hour's tables.
pub fn viewer<F: Tables + ?Sized>(frame: &F, links: &BTreeMap<u32, u32>, tags: &BTreeMap<u32, String>, country: &Country) -> io::Result<(u32, Resolution)> {
    resolve(perspective(frame, links), tags, country).ok_or_else(|| invalid(format!("hour {} records no country to view the map as", frame.at_hour())))
}

/// The viewing country from values: the client's stored viewpoint (`recorded`), the slots' tags,
/// and the country asked for. A tag compares case-insensitively and a slot must hold a country;
/// an absent one falls back to the recorded viewpoint, else the lowest slot. None when there is no country at all. The one rule
/// `perspective::viewer`, the vision layer and the engagement folds should share.
pub fn resolve(recorded: Option<u32>, tags: &BTreeMap<u32, String>, country: &Country) -> Option<(u32, Resolution)> {
    let default = || match recorded {
        Some(slot) => Some((slot, true)),
        None => tags.keys().next().map(|&slot| (slot, false)),
    };
    match country {
        Country::Recorded => default().map(|(slot, recorded)| (slot, if recorded { Resolution::Recorded } else { Resolution::NoPlayer })),
        Country::Tag(tag) => match tags.iter().find(|(_, name)| name.eq_ignore_ascii_case(tag)) {
            Some((&slot, _)) => Some((slot, Resolution::Requested)),
            None => default().map(|(slot, _)| (slot, Resolution::Absent(tag.clone()))),
        },
        Country::Slot { slot, .. } if tags.contains_key(slot) => Some((*slot, Resolution::Requested)),
        Country::Slot { .. } => default().map(|(slot, _)| (slot, Resolution::Absent(country.to_string()))),
    }
}
