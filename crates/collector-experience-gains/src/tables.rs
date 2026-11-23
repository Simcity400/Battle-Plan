use crate::{
    clock::qpc,
    experience_gains as producer,
    table_capture::{CaptureRows, grow_vec},
    table_conversion::table,
};
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::{rows::CountryExperienceGainRow, rows::TableId, rows::tables};
use replay_frame::frame::TableView;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::CountryExperienceGains];
pub type Capacities = usize;
pub struct Captured {
    rows: CaptureRows<CountryExperienceGainRow>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&0)
    }
    pub fn fail(&mut self, reason: &'static str) {
        self.rows.fail(Outcome::Unavailable { reason });
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            rows: CaptureRows::new(*capacity),
        }
    }
    pub fn reset(&mut self) {
        self.rows.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
    }
    pub fn grow(&mut self, capacity: &mut Capacities) {
        grow_vec(&mut self.rows.rows, *capacity);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        self.reset();
        let _reads = crate::read_accounting::table(TableId::CountryExperienceGains);
        let start = qpc();
        self.rows.outcome = match producer::drain(memory, base, state, &mut self.rows.rows) {
            Ok(len) => Outcome::Rows { len },
            Err(reason) => Outcome::Unavailable { reason },
        };
        [qpc().saturating_sub(start).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.rows.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::CountryExperienceGains, _>(
            self.rows.outcome,
            &self.rows.rows,
            Clone::clone,
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_frame::{
        frame::DecodedFrame, frame::FrameDecoder, frame::FrameEncoder, frame::Metrics,
    };

    #[test]
    fn gain_export_sorts_reordered_raw_aliases_and_refuses_duplicate_keys_before_encoding() {
        let alias = CountryExperienceGainRow {
            country: 10001,
            instance: u32::MAX,
            complete: 1,
            army_raw: i64::MAX,
            ..Default::default()
        };
        let other = CountryExperienceGainRow {
            country: 2,
            instance: 0,
            complete: 1,
            navy_raw: 7,
            ..Default::default()
        };
        let mut captured = Captured::with_capacities(&3);
        captured.rows.rows[..2].clone_from_slice(&[alias.clone(), other.clone()]);
        captured.rows.outcome = Outcome::Rows { len: 2 };
        let view = captured.tables().into_iter().next().unwrap();
        assert_eq!(captured.rows.rows[..2], [alias.clone(), other.clone()]);
        let mut views = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        views[TableId::CountryExperienceGains.index()] = view;
        let encoded = FrameEncoder::new()
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: views.map(Into::into),
                    metrics: Metrics::default(),
                },
                true,
            )
            .unwrap();
        let mut decoder = FrameDecoder::new();
        let decoded = decoder.apply(&encoded).unwrap();
        assert_eq!(
            decoded
                .complete::<tables::CountryExperienceGains>()
                .unwrap(),
            &[other, alias.clone()]
        );
        captured.rows.rows[1] = alias;
        assert!(
            matches!(captured.tables()[0], TableView::Unavailable { ref reason }
            if reason == "walker produced duplicate engine ids")
        );
        captured.rows.outcome = Outcome::Rows { len: 4 };
        assert!(
            matches!(captured.tables()[0], TableView::Unavailable { ref reason }
            if reason == "walker row count exceeds allocated buffer")
        );
    }
}
