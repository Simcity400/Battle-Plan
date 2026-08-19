use alloc::vec::Vec;

use crate::{
    ActiveTail, Header, ACTIVE_FLAG_BASE_REQUIRED_POINTS, ACTIVE_FLAG_LIVE_PROGRESS,
    COUNTRY_COUNT_ABSENT_SENTINEL, COUNTRY_FLAG_ACTIVE_RESEARCH, COUNTRY_FLAG_COMPLETED_TECH_COUNT,
    COUNTRY_FLAG_RESEARCH_SLOT_COUNT, COUNTRY_ROW_LEN, XUI09_COUNTRY_FLAG_DENSE, XUI09_FLAG_PATCH,
    XUI09_TAIL_MAGIC,
};

pub fn encode_header(header: &Header, out: &mut Vec<u8>) {
    out.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&header.country_count.to_le_bytes());
    out.extend_from_slice(&header.changed_rows.to_le_bytes());
    out.extend_from_slice(&header.status_flags.to_le_bytes());
    out.extend_from_slice(&header.progress_scale_raw.to_le_bytes());
    out.extend_from_slice(&header.live_scale_raw.to_le_bytes());
    out.extend_from_slice(&header.active_count.to_le_bytes());
}

pub fn country_row_bytes(
    tag_db_id: u32,
    completed_tech_count: Option<u32>,
    research_slot_count: Option<u32>,
    active_research_captured: bool,
) -> [u8; COUNTRY_ROW_LEN] {
    let mut flags = 0u32;
    if completed_tech_count.is_some() {
        flags |= COUNTRY_FLAG_COMPLETED_TECH_COUNT;
    }
    if research_slot_count.is_some() {
        flags |= COUNTRY_FLAG_RESEARCH_SLOT_COUNT;
    }
    if active_research_captured {
        flags |= COUNTRY_FLAG_ACTIVE_RESEARCH;
    }
    let mut row = [0u8; COUNTRY_ROW_LEN];
    row[0..4].copy_from_slice(&tag_db_id.to_le_bytes());
    row[4..8].copy_from_slice(&flags.to_le_bytes());
    row[8..12].copy_from_slice(
        &completed_tech_count
            .unwrap_or(COUNTRY_COUNT_ABSENT_SENTINEL)
            .to_le_bytes(),
    );
    row[12..16].copy_from_slice(
        &research_slot_count
            .unwrap_or(COUNTRY_COUNT_ABSENT_SENTINEL)
            .to_le_bytes(),
    );
    row
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct A29Provenance {
    pub authority: u32,
    pub localization_branch: u32,
    pub callback_sequence: u64,
    pub callback_thread_id: u32,
    pub producer_game_hour_raw: i32,
    pub callback_qpc_ticks: u64,
    pub qpc_frequency_hz: u64,
    pub source_record_sha256: [u8; 32],
    pub ui_action_gate_return_rva: u32,
    pub selected_present_candidate_count: u32,
    pub selected_present_ordinal: u32,
    pub selected_present_game_hour_raw: i32,
    pub selected_present_candidate_id: u64,
    pub selected_present_qpc_ticks: u64,
    pub selected_present_cutoff_qpc_ticks: u64,
    pub selected_present_candidate_set_sha256: [u8; 32],
    pub selected_present_receipt_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DerivedAuthority {
    pub authority: u32,
    pub producer_game_hour_raw: i32,
    pub penalty_factor_raw: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ahead<'a> {
    Observed {
        quartet: [i64; 4],
        provenance: &'a A29Provenance,
    },
    Derived {
        quartet: [i64; 4],
        authority: DerivedAuthority,
    },
}

impl Ahead<'_> {
    fn quartet(&self) -> [i64; 4] {
        match self {
            Ahead::Observed { quartet, .. } | Ahead::Derived { quartet, .. } => *quartet,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ActiveRow<'a> {
    pub tag_db_id: u32,
    pub slot_index: u32,
    pub tech_handle_raw: u32,
    pub tech_def_index_raw: u32,
    pub progress_raw: u32,
    pub live_progress_raw: Option<i64>,
    pub base_required_points_raw: Option<i64>,
    pub tech_name: Option<&'a str>,
    pub ahead: Option<Ahead<'a>>,
}

impl ActiveRow<'_> {
    fn name(&self) -> Option<&str> {
        self.tech_name.filter(|name| !name.is_empty())
    }

    pub fn flags(&self) -> u32 {
        let mut flags = 0u32;
        if self.live_progress_raw.is_some() {
            flags |= ACTIVE_FLAG_LIVE_PROGRESS;
        }
        if self.base_required_points_raw.is_some() {
            flags |= ACTIVE_FLAG_BASE_REQUIRED_POINTS;
        }
        for tail in ActiveTail::ALL {
            let present = match tail {
                ActiveTail::TechName => self.name().is_some(),
                ActiveTail::AheadQuartet => self.ahead.is_some(),
                ActiveTail::ObservedAuthority => {
                    matches!(self.ahead, Some(Ahead::Observed { .. }))
                }
                ActiveTail::DerivedAuthority => matches!(self.ahead, Some(Ahead::Derived { .. })),
            };
            if present {
                flags |= tail.flag_bit();
            }
        }
        flags
    }
}

pub fn encode_active_row(row: &ActiveRow<'_>, out: &mut Vec<u8>) {
    let flags = row.flags();
    out.extend_from_slice(&row.tag_db_id.to_le_bytes());
    out.extend_from_slice(&row.slot_index.to_le_bytes());
    out.extend_from_slice(&row.tech_handle_raw.to_le_bytes());
    out.extend_from_slice(&row.tech_def_index_raw.to_le_bytes());
    out.extend_from_slice(&row.progress_raw.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&row.live_progress_raw.unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&row.base_required_points_raw.unwrap_or(0).to_le_bytes());
    for tail in ActiveTail::ALL {
        match tail {
            ActiveTail::TechName => {
                if let Some(name) = row.name() {
                    encode_prefixed_string(name, out);
                }
            }
            ActiveTail::AheadQuartet => {
                if let Some(ahead) = row.ahead {
                    for value in ahead.quartet() {
                        out.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
            ActiveTail::ObservedAuthority => {
                if let Some(Ahead::Observed { provenance, .. }) = row.ahead {
                    encode_a29_provenance(provenance, out);
                }
            }
            ActiveTail::DerivedAuthority => {
                if let Some(Ahead::Derived { authority, .. }) = row.ahead {
                    out.extend_from_slice(&authority.authority.to_le_bytes());
                    out.extend_from_slice(&authority.producer_game_hour_raw.to_le_bytes());
                    out.extend_from_slice(&authority.penalty_factor_raw.to_le_bytes());
                }
            }
        }
    }
}

fn encode_a29_provenance(provenance: &A29Provenance, out: &mut Vec<u8>) {
    out.extend_from_slice(&provenance.authority.to_le_bytes());
    out.extend_from_slice(&provenance.localization_branch.to_le_bytes());
    out.extend_from_slice(&provenance.callback_sequence.to_le_bytes());
    out.extend_from_slice(&provenance.callback_thread_id.to_le_bytes());
    out.extend_from_slice(&provenance.producer_game_hour_raw.to_le_bytes());
    out.extend_from_slice(&provenance.callback_qpc_ticks.to_le_bytes());
    out.extend_from_slice(&provenance.qpc_frequency_hz.to_le_bytes());
    out.extend_from_slice(&provenance.source_record_sha256);
    out.extend_from_slice(&provenance.ui_action_gate_return_rva.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_candidate_count.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_ordinal.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_game_hour_raw.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_candidate_id.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_qpc_ticks.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_cutoff_qpc_ticks.to_le_bytes());
    out.extend_from_slice(&provenance.selected_present_candidate_set_sha256);
    out.extend_from_slice(&provenance.selected_present_receipt_sha256);
}

fn encode_prefixed_string(value: &str, out: &mut Vec<u8>) {
    let bytes = value.as_bytes();
    let len = if bytes.len() > u16::MAX as usize {
        u16::MAX as usize
    } else {
        bytes.len()
    };
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..len]);
}

pub fn xui09_country_vector_body(states: &[(u32, u8)], completed: &[u32]) -> Vec<u8> {
    let dense = states
        .iter()
        .enumerate()
        .all(|(position, (def_index, _))| *def_index as usize == position);
    let mut flags = 0u32;
    if dense {
        flags |= XUI09_COUNTRY_FLAG_DENSE;
    }
    let mut bytes =
        Vec::with_capacity(12 + states.len() * if dense { 1 } else { 5 } + completed.len() * 4);
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&(states.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(completed.len() as u32).to_le_bytes());
    for (def_index, state) in states {
        if !dense {
            bytes.extend_from_slice(&def_index.to_le_bytes());
        }
        bytes.push(*state);
    }
    for def_index in completed {
        bytes.extend_from_slice(&def_index.to_le_bytes());
    }
    bytes
}

pub struct Xui09Tail<D, M, R, V> {
    pub patch: bool,
    pub dictionary: D,
    pub measured: M,
    pub refused: R,
    pub vectors: V,
}

pub fn encode_xui09_tail<'a, D, M, R, V>(tail: Xui09Tail<D, M, R, V>, out: &mut Vec<u8>)
where
    D: ExactSizeIterator<Item = (u32, &'a str)>,
    M: ExactSizeIterator<Item = u32>,
    R: ExactSizeIterator<Item = (u32, u32)>,
    V: ExactSizeIterator<Item = (u32, &'a [u8])>,
{
    let Xui09Tail {
        patch,
        dictionary,
        measured,
        refused,
        vectors,
    } = tail;
    let mut flags = 0u32;
    if patch {
        flags |= XUI09_FLAG_PATCH;
    }
    out.extend_from_slice(&XUI09_TAIL_MAGIC.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(dictionary.len() as u32).to_le_bytes());
    out.extend_from_slice(&(measured.len() as u32).to_le_bytes());
    out.extend_from_slice(&(refused.len() as u32).to_le_bytes());
    out.extend_from_slice(&(vectors.len() as u32).to_le_bytes());
    for (def_index, id) in dictionary {
        out.extend_from_slice(&def_index.to_le_bytes());
        encode_prefixed_string(id, out);
    }
    for tag_db_id in measured {
        out.extend_from_slice(&tag_db_id.to_le_bytes());
    }
    for (tag_db_id, reason) in refused {
        out.extend_from_slice(&tag_db_id.to_le_bytes());
        out.extend_from_slice(&reason.to_le_bytes());
    }
    for (tag_db_id, body) in vectors {
        out.extend_from_slice(&tag_db_id.to_le_bytes());
        out.extend_from_slice(body);
    }
}
