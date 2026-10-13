//! A save has no direct representation of the actual reserved receiver's
//! current diplomacy matrix associations. Saved relation payloads continue to
//! compare through the ordinary relation tables; their equality is not a
//! witness for these cached receiver-cell inputs.

use super::*;

const LABEL: &str = "ReservedCountryNaming.reverse_visibility";

pub(super) fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let [row] = frame.complete::<tables::ReservedCountryNaming>()? else {
        return Err(invalid(
            "reserved reverse visibility requires one naming row",
        ));
    };
    let Some(packet) = report.recorded(&row.reverse_visibility, LABEL, LABEL) else {
        return Ok(());
    };
    let Some(packet) = packet else {
        report.limitation(
            "actual reserved receiver reverse visibility was unavailable at capture; no empty matrix or false disclosure is inferred",
            LABEL,
        );
        return Ok(());
    };
    if !packet.sample_hour.fresh_at(frame.hour) {
        return Err(invalid(format!(
            "{LABEL}.sample_hour {:?} differs from oracle hour {}",
            packet.sample_hour, frame.hour,
        )));
    }
    if packet.cells.len() > i32::MAX as usize {
        return Err(invalid(
            "reserved reverse matrix count exceeds the native signed domain",
        ));
    }
    report.limitation(
        "the actual reserved receiver's diplomacy matrix count/order and each cell's raw target association have no direct text-save counterpart; a matched same-hour native witness is required, including literal target0 versus positive target aliases to canonical0",
        format!("{LABEL}: {} native matrix ordinals", packet.cells.len()),
    );
    report.limitation(
        "reverse cells' war-pointer presence and raw retirement bytes are runtime receiver status; saved WarRelations payload equality cannot certify the current cell association or an active/retired transition, so a matched same-hour native witness is required",
        format!("{LABEL}.war"),
    );
    report.limitation(
        "reverse cells' subject pointer/type/raw-party association and native first/second-party OR gates are runtime receiver inputs; saved Subjects payload equality cannot certify their current association, so a matched same-hour native witness is required",
        format!("{LABEL}.subject"),
    );
    report.limitation(
        "reverse cells' military-access pointer presence is runtime receiver status; saved relation payloads cannot certify the current cell's exact pointer presence, so a matched same-hour full-width native witness is required",
        format!("{LABEL}.military_access"),
    );
    for (ordinal, cell) in packet.cells.iter().enumerate() {
        let Some(cell) = cell else {
            report.limitation(
                "a reverse receiver cell was unavailable at capture; its ordinal is retained and no null relation or false disclosure is inferred",
                format!("{LABEL}.cells[{ordinal}]"),
            );
            continue;
        };
        if cell.war.is_none() {
            report.limitation(
                "reverse war-pointer presence/retirement operands were unavailable at capture; faction equality does not certify a clear war",
                format!("{LABEL}.cells[{ordinal}].war"),
            );
        }
        if cell.target.is_none() {
            report.limitation(
                "this reverse cell's raw target was unreached or unavailable; no literal target0 or positive alias is inferred",
                format!("{LABEL}.cells[{ordinal}].target"),
            );
        }
        if cell.subject.is_none() {
            report.limitation(
                "this reverse cell's subject operands were unreached or unavailable; a known war/faction gate may bypass their demand, but their absence does not prove a null subject",
                format!("{LABEL}.cells[{ordinal}].subject"),
            );
        }
        if cell.military_access.is_some_and(|presence| presence > 1) {
            return Err(invalid(format!(
                "{LABEL}.cells[{ordinal}].military_access is not a native pointer presence",
            )));
        }
        if cell.military_access.is_none() {
            report.limitation(
                "this reverse cell's military-access pointer was unreached or unavailable; a known war or flag-zero query may bypass its demand, but absence does not prove null access",
                format!("{LABEL}.cells[{ordinal}].military_access"),
            );
        }
    }
    report.compared(LABEL, 0);
    Ok(())
}

#[cfg(test)]
#[path = "reverse_visibility_tests.rs"]
mod tests;
