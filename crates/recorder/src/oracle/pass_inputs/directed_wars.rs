//! Structural self-check of hourly sparse direct-war observations. This is
//! independent of saved-country comparisons and is NOT a save oracle for the
//! native matrix pointer/retirement predicate or completeness certificate.
use super::{Report, invalid};
use replay::{frame::DecodedFrame, rows::tables};
use std::io;

pub(super) fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    for row in frame.complete::<tables::CountryCounterInputs>()? {
        let label = format!("CountryCounterInputs {}.directed_wars", row.country);
        let Some(packet) = report.recorded(
            &row.directed_wars,
            "CountryCounterInputs.directed_wars",
            &label,
        ) else {
            continue;
        };
        let Some(packet) = packet else {
            report.limitation("directed-war owner/header/arena capture refused; no clear war, zero cells or complete domain is inferred",&label);
            continue;
        };
        report.attempt(&format!("{label}.owner_hour_domains"), |_| {
            if row.country == 0 {
                return Err(invalid(
                    "actual-zero directed wars must reuse existing reserved reverse visibility",
                ));
            }
            let mut owners = frame
                .complete::<tables::Countries>()?
                .iter()
                .filter(|owner| owner.id == row.country);
            let owner = owners
                .next()
                .ok_or_else(|| invalid("directed-war receiver country missing"))?;
            if owners.next().is_some() {
                return Err(invalid("directed-war receiver country duplicated"));
            }
            packet
                .admit(frame.hour, owner.id as i32, owner.canonical_slot)
                .map(|_| ())
                .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))
        });
        report.limitation("exact directed pointer/nonretired-byte predicates and sparse active-domain completeness have no proved text-save counterpart; dates, symmetric WarRelations and enemy lists are not equivalent. Requires an independent matched same-boundary native/write witness, including alias/literal0, omitted and refused targets",&label);
        if !packet.complete_active_domain {
            report.limitation("partial observed domain: every absent target remains unknown; exhaustive direct/generic slot writers, candidate completeness and endpoint lifetime are still held",format!("{label}.complete_active_domain"));
        }
        if !packet.refused_slots.is_empty() {
            report.limitation("independent target capture refusals remain unknown even in an otherwise certified complete active domain",format!("{label}.refused_slots {:?}",packet.refused_slots));
        }
    }
    Ok(())
}
