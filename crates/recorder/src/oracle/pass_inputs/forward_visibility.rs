//! Forward status0 operands have no proved text-save counterpart. These are
//! structural self-checks and explicit native-witness limitations, independent
//! of CountryCounterInputs' unchanged saved-country comparisons. The existing
//! hourly table dates the packet; it contains no separate sample-hour scalar.
use super::{Report, invalid};
use replay::{frame::DecodedFrame, rows::tables};
use std::io;

const LABEL: &str = "CountryCounterInputs.forward_visibility";

pub(super) fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    for row in frame.complete::<tables::CountryCounterInputs>()? {
        let label = format!("CountryCounterInputs {}.forward_visibility", row.country);
        let Some(packet) = report.recorded(&row.forward_visibility, LABEL, &label) else {
            continue;
        };
        let Some(packet) = packet else {
            report.limitation(
                "forward status0 visibility packet was unavailable at capture; no clear war, null military access, null subject or false visibility is inferred",
                &label,
            );
            continue;
        };
        // A bad receiver cannot grant general actual-Country0 support. This
        // independent admission does not suppress the packet's field checks.
        report.attempt(&format!("{label}.receiver"), |_| {
            positive_receiver(frame, row.country)
        });
        report.limitation(
            "forward status0 raw target association has no proved text-save counterpart; a matched same-hour native witness must distinguish literal0 from exact positive aliases to canonical0; this structural self-check resolves no target link",
            match packet.target {
                Some(raw) => format!("{label}.target raw {raw}"),
                None => format!("{label}.target"),
            },
        );
        if packet.target.is_none() {
            report.limitation(
                "forward raw target was unreached or unavailable; native early war/military arms need not reach it and no literal0 or alias is inferred",
                format!("{label}.target"),
            );
        }
        report.limitation(
            "forward status0 war-pointer presence and pointed raw retired byte have no proved text-save counterpart; saved WarRelations equality does not certify this cell association, so a matched same-hour full-width pointer/retired-byte native witness is required",
            format!("{label}.war"),
        );
        match packet.war {
            Some([presence, retired]) => {
                report.attempt(&format!("{label}.war"), |_| {
                    // Retirement is an exact byte, not a boolean domain.
                    // A null pointer has no pointed retirement byte.
                    if presence > 1 || (presence == 0 && retired != 0) {
                        Err(invalid("forward visibility war is not native null or nonnull presence with an exact retired byte"))
                    } else {
                        Ok(())
                    }
                });
            }
            None => report.limitation(
                "forward war-pointer presence/retired byte was unreached or unavailable; faction equality does not certify a clear war",
                format!("{label}.war"),
            ),
        }
        report.limitation(
            "forward status0 military-access presence has no proved text-save counterpart; saved relation equality cannot certify its exact eight-byte pointer presence, so a matched same-hour full-width native witness is required",
            format!("{label}.military_access"),
        );
        match packet.military_access {
            Some(presence) => {
                report.attempt(&format!("{label}.military_access"), |_| {
                    if presence > 1 {
                        Err(invalid("forward visibility military access is not a native pointer presence"))
                    } else {
                        Ok(())
                    }
                });
            }
            None => report.limitation(
                "forward military-access pointer was unreached or unavailable; a known war or flag-zero arm may bypass it, but absence does not prove null access",
                format!("{label}.military_access"),
            ),
        }
        report.limitation(
            "forward status0 subject pointer/type and ordered raw first/second-party OR operands have no proved text-save counterpart; saved Subjects equality cannot certify their association or short-circuits, so a matched same-hour native witness is required; no target or party alias is resolved here",
            format!("{label}.subject"),
        );
        match packet.subject {
            None => report.limitation(
                "forward subject operands were unreached or unavailable; known early arms may bypass them, but their absence does not prove a null subject",
                format!("{label}.subject"),
            ),
            Some(subject) => {
                report.attempt(&format!("{label}.subject"), |_| {
                    if subject.relation_type.is_none() && subject.parties.is_some() {
                        Err(invalid("forward visibility native-null subject cannot contain pointed party operands"))
                    } else {
                        Ok(())
                    }
                });
                if subject.relation_type.is_some() {
                    match subject.parties {
                        None => report.limitation(
                            "forward subject parties were unreached or unavailable; a recorded non-subject type can skip both reads and no missing party becomes zero",
                            format!("{label}.subject.parties"),
                        ),
                        Some(parties) => {
                            for (ordinal, party) in parties.iter().enumerate() {
                                if party.is_none() {
                                    report.limitation(
                                        "forward subject party was unreached or unavailable; native ordered OR may skip a later party and no missing word becomes zero",
                                        format!("{label}.subject.parties[{ordinal}]"),
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    // None of these native-only self-checks counts as a saved comparison.
    Ok(())
}

fn positive_receiver(frame: &DecodedFrame, raw: u32) -> io::Result<()> {
    if raw == 0 {
        return Err(invalid(
            "forward visibility receiver is not a positive country",
        ));
    }
    let mut owners = frame
        .complete::<tables::Countries>()?
        .iter()
        .filter(|country| country.id == raw);
    let owner = owners
        .next()
        .ok_or_else(|| invalid("forward visibility receiver is absent from Countries"))?;
    if owners.next().is_some() || owner.canonical_slot == 0 {
        return Err(invalid(
            "forward visibility receiver has no unique positive canonical country",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "forward_visibility_tests.rs"]
mod tests;
