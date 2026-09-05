use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReadHours {
    pub before: Option<i32>,
    pub after: Option<i32>,
}

impl ReadHours {
    pub fn observed_game_hour_raw(self) -> Option<i32> {
        self.before.filter(|hour| Some(*hour) == self.after)
    }

    pub fn admits(self, frame_hour: i32) -> bool {
        self.observed_game_hour_raw() == Some(frame_hour)
    }

    pub fn refusal(self, frame_hour: i32) -> Option<ReadRefusal> {
        match (self.before, self.after) {
            (Some(a), Some(b)) if a != b => Some(ReadRefusal::Torn),
            (Some(a), Some(_)) if a != frame_hour => Some(ReadRefusal::HourLabelMismatch),
            (Some(_), Some(_)) => None,
            _ => Some(ReadRefusal::ClockUnreadable),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct FrameHourCustody {
    pub snapshot: ReadHours,
    pub carrier: ReadHours,
}

impl FrameHourCustody {
    pub fn witnessed(hour: i32) -> Self {
        let hours = ReadHours {
            before: Some(hour),
            after: Some(hour),
        };
        Self {
            snapshot: hours,
            carrier: hours,
        }
    }

    pub(crate) fn encode(self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"D55H");
        for hour in [
            self.snapshot.before,
            self.snapshot.after,
            self.carrier.before,
            self.carrier.after,
        ] {
            out.extend_from_slice(&hour.unwrap_or(-1).to_le_bytes());
        }
    }

    pub(crate) fn decode(payload: &[u8], cursor: &mut usize) -> Result<Self, String> {
        if payload.get(*cursor..*cursor + 4) != Some(b"D55H".as_slice()) {
            return Err("D55 hour custody missing".into());
        }
        *cursor += 4;
        let snapshot = ReadHours {
            before: decode_hour(payload, cursor)?,
            after: decode_hour(payload, cursor)?,
        };
        let carrier = ReadHours {
            before: decode_hour(payload, cursor)?,
            after: decode_hour(payload, cursor)?,
        };
        Ok(Self { snapshot, carrier })
    }
}

pub(crate) fn decode_hour(payload: &[u8], cursor: &mut usize) -> Result<Option<i32>, String> {
    let bytes = payload
        .get(*cursor..*cursor + 4)
        .ok_or("D55 observation hour truncated")?;
    *cursor += 4;
    match i32::from_le_bytes(bytes.try_into().unwrap()) {
        -1 => Ok(None),
        hour if hour >= 0 => Ok(Some(hour)),
        _ => Err("D55 observation hour invalid".into()),
    }
}

#[cfg(windows)]
pub(crate) fn current_game_hour_raw() -> Option<i32> {
    use core::{ffi::c_void, mem, ptr};
    use windows_sys::Win32::System::{
        Diagnostics::Debug::ReadProcessMemory, LibraryLoader::GetModuleHandleW,
        Threading::GetCurrentProcess,
    };
    fn read<T: Copy + Default>(address: usize) -> Option<T> {
        let mut value = T::default();
        let mut copied = 0;
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                address as *const c_void,
                (&mut value as *mut T).cast(),
                mem::size_of::<T>(),
                &mut copied,
            )
        };
        (ok != 0 && copied == mem::size_of::<T>()).then_some(value)
    }
    let base = unsafe { GetModuleHandleW(ptr::null()) as usize };
    if base == 0 {
        return None;
    }
    let state = read::<usize>(base.checked_add(0x0331_6E90)?)?;
    if state == 0 {
        return None;
    }
    let first = read::<i32>(state.checked_add(0x468)?)?;
    let second = read::<i32>(state.checked_add(0x468)?)?;
    (first >= 0 && first == second).then_some(first)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadRefusal {
    Torn,
    HourLabelMismatch,
    ClockUnreadable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublishedHookCustody {
    pub frame_game_hour_raw: i32,
    pub read_hours: FrameHourCustody,
    pub snapshot_refusal: Option<ReadRefusal>,
    pub carrier_refusal: Option<ReadRefusal>,
    pub event_hour_refusals: Vec<EventHourRefusal>,
    pub terminal_record_row_count: usize,
    pub width_observation_row_count: usize,
    pub current_loss_observation_row_count: usize,
    pub panel_observation_row_count: usize,
}

impl PublishedHookCustody {
    pub fn admit(
        decoded: &crate::DecodedDetailEventsPayload,
        frame_game_hour_raw: i32,
    ) -> Result<Self, String> {
        crate::validate_frame_hour(decoded, frame_game_hour_raw)?;
        Ok(Self {
            frame_game_hour_raw,
            read_hours: decoded.hour_custody,
            snapshot_refusal: decoded.hour_custody.snapshot.refusal(frame_game_hour_raw),
            carrier_refusal: decoded.hour_custody.carrier.refusal(frame_game_hour_raw),
            event_hour_refusals: decoded
                .terminal_record_rows
                .iter()
                .map(|row| row.observed_game_hour_raw)
                .enumerate()
                .map(|(index, hour)| (HookFamily::Terminal, index, hour))
                .chain(
                    decoded
                        .width_observation_rows
                        .iter()
                        .map(|row| row.observed_game_hour_raw)
                        .enumerate()
                        .map(|(index, hour)| (HookFamily::Width, index, hour)),
                )
                .chain(
                    decoded
                        .current_loss_observation_rows
                        .iter()
                        .map(|row| row.observed_game_hour_raw)
                        .enumerate()
                        .map(|(index, hour)| (HookFamily::CurrentLoss, index, hour)),
                )
                .chain(
                    decoded
                        .panel_observation_rows
                        .iter()
                        .map(|row| row.observed_game_hour_raw)
                        .enumerate()
                        .map(|(index, hour)| (HookFamily::Panel, index, hour)),
                )
                .filter_map(|(family, row_index, hour)| {
                    hour.is_none().then_some(EventHourRefusal {
                        family,
                        row_index,
                        reason: EventHourRefusalReason::ObservationHourUnavailable,
                    })
                })
                .collect(),
            terminal_record_row_count: decoded.terminal_record_rows.len(),
            width_observation_row_count: decoded.width_observation_rows.len(),
            current_loss_observation_row_count: decoded.current_loss_observation_rows.len(),
            panel_observation_row_count: decoded.panel_observation_rows.len(),
        })
    }
}

pub(crate) fn witnessed_event_hour(before: Option<i32>, after: Option<i32>) -> Option<i32> {
    before.filter(|hour| Some(*hour) == after)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookFamily {
    Terminal,
    Width,
    CurrentLoss,
    Panel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventHourRefusalReason {
    ObservationHourUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EventHourRefusal {
    pub family: HookFamily,
    pub row_index: usize,
    pub reason: EventHourRefusalReason,
}
