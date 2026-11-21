use super::Report;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::io;

pub(super) fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let mut derived = 0_usize;
    for row in frame.complete::<tables::CountryCounterInputs>()? {
        let label = format!("CountryCounterInputs {}.directed_wars", row.country);
        if let Some(None) = report.recorded(
            &row.directed_wars,
            "CountryCounterInputs.directed_wars",
            &label,
        ) {
            derived += 1;
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
