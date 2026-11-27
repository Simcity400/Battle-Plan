pub mod air_ledger;
pub mod air_aces;
pub mod air_missions;
pub mod air_statistics;
pub mod air_superiority;
pub mod aircraft_catalogue;
pub mod regional_weather;

pub(crate) fn status_rows<'a>(
    status: replay_columnar::manifest::Status,
    reasons: &'a std::collections::HashMap<u32, String>,
    mode: replay_data::scope::CaptureMode,
) -> dashboard_text::metadata::RecordedRows<'a, ()> {
    use dashboard_text::metadata::RecordedRows;
    use replay_columnar::manifest::Status;
    match status {
        Status::Complete => RecordedRows::Complete(()),
        Status::Truncated { missing } => RecordedRows::Truncated { rows: (), missing },
        Status::Unavailable { reason } => RecordedRows::Unavailable {
            reason: reasons.get(&reason).map_or("", String::as_str),
        },
        Status::NotRecorded => RecordedRows::NotRecorded { mode },
    }
}
