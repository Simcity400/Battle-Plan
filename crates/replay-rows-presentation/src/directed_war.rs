
use super::*;

type Fact<T> = Result<T, String>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct WarCell {
    pub target_slot: u32,
    pub active: bool,
}
value_struct!(WarCell {
    target_slot: u32,
    active: bool
});

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct DirectedWars {
    pub sample_hour: PacketHour,
    pub receiver_raw_id: i32,
    pub receiver_slot: u32,
    pub matrix_count: u32,
    pub complete_active_domain: bool,
    pub cells: Vec<WarCell>,
    pub refused_slots: Vec<u32>,
}

value_struct!(DirectedWars { sample_hour: PacketHour, receiver_raw_id: i32, receiver_slot: u32, matrix_count: u32, complete_active_domain: bool, cells: Vec<WarCell>, refused_slots: Vec<u32> });
impl OpaqueValue for DirectedWars {}

#[derive(Clone, Copy, Debug)]
pub struct DirectedWarContext<'a> {
    packet: &'a DirectedWars,
}

impl DirectedWars {
    fn validate(&self) -> Fact<()> {
        if self.matrix_count == 0
            || self.matrix_count > i32::MAX as u32
            || self.receiver_slot >= self.matrix_count
            || !self
                .cells
                .windows(2)
                .all(|p| p[0].target_slot < p[1].target_slot)
            || !self.refused_slots.windows(2).all(|p| p[0] < p[1])
            || self
                .cells
                .iter()
                .any(|c| c.target_slot >= self.matrix_count)
            || self.refused_slots.iter().any(|&s| s >= self.matrix_count)
            || self
                .cells
                .iter()
                .any(|c| self.refused_slots.binary_search(&c.target_slot).is_ok())
        {
            return Err("directed war sparse domains invalid or ambiguous".into());
        }
        Ok(())
    }

    pub fn admit(
        &self,
        hour: i32,
        receiver_raw_id: i32,
        receiver_slot: u32,
    ) -> Fact<DirectedWarContext<'_>> {
        self.validate()?;
        if !self.sample_hour.fresh_at(hour) {
            return Err("directed war sample hour mismatch".into());
        }
        if (self.receiver_raw_id, self.receiver_slot) != (receiver_raw_id, receiver_slot) {
            return Err("directed war receiver differs from current CountryLinks".into());
        }
        Ok(DirectedWarContext { packet: self })
    }
}

impl DirectedWarContext<'_> {
    pub fn pointer_active(&self, target_slot: u32) -> Fact<bool> {
        let p = self.packet;
        if target_slot >= p.matrix_count {
            return Err("directed war target outside native matrix".into());
        }
        if p.refused_slots.binary_search(&target_slot).is_ok() {
            return Err(format!("directed war target {target_slot} capture refused"));
        }
        match p
            .cells
            .binary_search_by_key(&target_slot, |cell| cell.target_slot)
        {
            Ok(index) => Ok(p.cells[index].active),
            Err(_) if p.complete_active_domain => Ok(false),
            Err(_) => Err(format!(
                "directed war target {target_slot} has no certified observation"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_rows::{
        codec::Decoder,
        rows::{Recorded, RowCodec},
        value::Value,
    };
    fn packet() -> DirectedWars {
        DirectedWars {
            sample_hour: PacketHour::Host,
            receiver_raw_id: 10,
            receiver_slot: 1,
            matrix_count: 9,
            complete_active_domain: false,
            cells: vec![
                WarCell {
                    target_slot: 0,
                    active: true,
                },
                WarCell {
                    target_slot: 3,
                    active: false,
                },
            ],
            refused_slots: vec![5],
        }
    }
    #[test]
    fn partial_unknown_refused_and_exact_values_survive_codec() {
        let p = packet();
        let mut bytes = Vec::new();
        Value::encode(&p, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = DirectedWars::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, p);
        let context = decoded.admit(100, 10, 1).unwrap();
        assert_eq!(context.pointer_active(0), Ok(true));
        assert_eq!(context.pointer_active(3), Ok(false));
        assert!(context.pointer_active(2).is_err());
        assert!(context.pointer_active(5).is_err());
        assert!(context.pointer_active(9).is_err());
    }
    #[test]
    fn directed_sparse_native_extent_above_u16_survives_codec_and_admission() {
        let mut packet = packet();
        packet.matrix_count = 65_537;
        packet.cells.push(WarCell {
            target_slot: 65_536,
            active: true,
        });
        let mut bytes = Vec::new();
        Value::encode(&packet, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let decoded = DirectedWars::decode(&mut input).unwrap();
        input.finish().unwrap();
        assert_eq!(decoded, packet);
        let admitted = decoded.admit(100, 10, 1).unwrap();
        assert_eq!(admitted.pointer_active(65_536), Ok(true));
        assert!(admitted.pointer_active(65_537).is_err());
        assert!(admitted.pointer_active(2).is_err());
    }
    #[test]
    fn directed_sparse_admission_keeps_the_native_signed_header_domain() {
        let mut packet = packet();
        packet.matrix_count = i32::MAX as u32;
        assert!(packet.admit(100, 10, 1).is_ok());
        for invalid in [i32::MAX as u32 + 1, u32::MAX] {
            packet.matrix_count = invalid;
            assert!(packet.admit(100, 10, 1).is_err());
        }
    }
    #[test]
    fn stale_owner_and_malformed_domains_never_admit() {
        let p = packet();
        assert!(p.admit(99, 10, 1).is_ok());
        let stamped = DirectedWars {
            sample_hour: PacketHour::Stamped(100),
            ..packet()
        };
        assert!(stamped.admit(100, 10, 1).is_ok());
        assert!(stamped.admit(99, 10, 1).is_err());
        assert!(p.admit(100, 11, 1).is_err());
        assert!(p.admit(100, 10, 2).is_err());
        let mut p = packet();
        p.refused_slots = vec![0];
        assert!(p.admit(100, 10, 1).is_err());
        let mut p = packet();
        p.cells.push(WarCell {
            target_slot: 0,
            active: true,
        });
        assert!(p.admit(100, 10, 1).is_err());
    }
    #[test]
    fn directed_suffix_delta_changes_only_its_bit() {
        let old = CountryCounterInputRow {
            country: 10,
            ..Default::default()
        };
        let new = CountryCounterInputRow {
            directed_wars: Recorded::Value(Some(packet())),
            ..old.clone()
        };
        let delta = new.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 18);
        assert_eq!(delta.apply(Some(old)).unwrap(), new);
        let mut bytes = Vec::new();
        new.encode_fields(&mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        assert_eq!(
            CountryCounterInputRow::decode_fields(new.key(), &mut input).unwrap(),
            new
        );
        input.finish().unwrap();
    }
    #[test]
    fn current_directed_wars_clear_without_changing_air_combat_columns() {
        let before = CountryCounterInputRow {
            country: 10,
            directed_wars: Recorded::Value(Some(packet())),
            air_combat_modifiers_known: true.into(),
            air_attack_factor: Some(i64::MIN).into(),
            air_defence_factor: Some(i64::MAX).into(),
            ..Default::default()
        };
        let after = CountryCounterInputRow {
            directed_wars: Recorded::Value(None),
            ..before.clone()
        };
        for row in [&before, &after] {
            let mut bytes = Vec::new();
            row.encode_fields(&mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            assert_eq!(
                CountryCounterInputRow::decode_fields(row.key(), &mut input).unwrap(),
                *row
            );
            input.finish().unwrap();
        }
        let delta = after.delta(Some(&before)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 18);
        assert_eq!(delta.apply(Some(before)).unwrap(), after);
        assert_eq!(after.not_recorded_fields(), 0);
    }
}
