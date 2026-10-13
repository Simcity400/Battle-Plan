//! Structural self-check of the sparse direct-war packets recordings through
//! format 101 stored. Current recordings store none: playback derives every
//! cell from the hour's WarRelations, which the war oracle compares with the
//! save. This is NOT a save oracle for an older packet's matrix predicate.
use super::{Report, invalid};
use replay::{frame::DecodedFrame, rows::tables};
use std::io;

pub(super) fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let mut derived = 0_usize;
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
            derived += 1;
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
        report.limitation("an older packet's pointer/nonretired-byte cells have no text-save counterpart; the same hour's WarRelations derives every cell (directed_war_derive) but is a separate capture, and dates and enemy lists are not equivalent",&label);
        if !packet.complete_active_domain {
            report.limitation("partial observed domain: every absent target remains unknown; exhaustive direct/generic slot writers, candidate completeness and endpoint lifetime are still held",format!("{label}.complete_active_domain"));
        }
        if !packet.refused_slots.is_empty() {
            report.limitation("independent target capture refusals remain unknown even in an otherwise certified complete active domain",format!("{label}.refused_slots {:?}",packet.refused_slots));
        }
    }
    if derived != 0 {
        report.limitation(
            "no direct-war packet: playback derives these receivers' cells from the hour's WarRelations (current recordings store none; an older recording's None was a refused capture)",
            format!("CountryCounterInputs.directed_wars ({derived} receivers)"),
        );
    }
    Ok(())
}
