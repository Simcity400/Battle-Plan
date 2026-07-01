use crate::domain47_characters_leaders_cabinet_live as compact;
use crate::extractor::{encode_frame_payload, LaneFrame};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaneCollectorModule {
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectedDomainRecord {
    pub domain_id: u16,
    pub domain_name: &'static str,
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub payload: Vec<u8>,
    pub wrapped_record: Vec<u8>,
}

impl LaneCollectorModule {
    pub fn new(sequence: u64) -> Self {
        Self { sequence }
    }

    pub fn collect_frame(
        &mut self,
        game_hour_raw: i32,
        frame: &LaneFrame,
    ) -> CollectedDomainRecord {
        let sequence = self.sequence;
        self.sequence = self.sequence.saturating_add(1);
        let payload = encode_frame_payload(frame);
        let wrapped_record = compact::encode_compact_data_record(sequence, game_hour_raw, &payload);
        CollectedDomainRecord {
            domain_id: compact::DOMAIN_ID,
            domain_name: compact::DOMAIN_NAME,
            sequence,
            game_hour_raw,
            payload,
            wrapped_record,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extractor::{AdvisorRecord, LaneFrame};

    #[test]
    fn collector_module_wraps_domain47_record() {
        let mut frame = LaneFrame::new(15);
        frame.appointed_advisors.push(AdvisorRecord {
            country_tag_db_id: 2,
            char_id: 1458,
            slot: Some("political_advisor".to_string()),
            ledger: Some("political_advisor".to_string()),
            idea_token: "ENG_maxwell_aitken".to_string(),
        });

        let mut module = LaneCollectorModule::new(42);
        let record = module.collect_frame(1946051909, &frame);
        assert_eq!(record.domain_id, compact::DOMAIN_ID);
        assert_eq!(record.sequence, 42);
        assert_eq!(record.domain_name, compact::DOMAIN_NAME);
        assert_eq!(&record.wrapped_record[0..4], compact::COMPACT_RECORD_MAGIC);
        assert_eq!(
            u16::from_le_bytes(record.wrapped_record[20..22].try_into().unwrap()),
            compact::DOMAIN_ID
        );
        assert_eq!(module.sequence, 43);
    }
}
