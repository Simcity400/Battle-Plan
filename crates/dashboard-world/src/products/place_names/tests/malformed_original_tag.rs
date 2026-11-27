use super::*;
use dashboard_base::test_fixtures::session;
use replay_data::rows::{CampaignRow, ClockRow, CountryRow, DeltaRow, Field, Opaque, RowCodec, column};
use replay_frame::branches::{BranchReason, BranchStart, Parentage};
use replay_journal::synthetic::{Input, Sample};

#[derive(Clone, Default, PartialEq, Eq)]
struct Appearance(CountryAppearanceRow, bool);

impl RowCodec for Appearance {
    const KEY_BITS: u32 = CountryAppearanceRow::KEY_BITS;

    fn key(&self) -> u128 { self.0.key() }
    fn try_clone(&self) -> io::Result<Self> { self.0.try_clone().map(|row| Self(row, self.1)) }
    fn try_clone_validated(&self) -> io::Result<Self> { self.try_clone() }
    fn heap_bytes(&self) -> usize { self.0.heap_bytes() }
    fn encoded_bound(&self) -> io::Result<usize> { self.0.encoded_bound() }
    fn snapshot_bytes(&self) -> io::Result<usize> { self.0.snapshot_bytes() }
    fn check_key_fields(&self) -> io::Result<()> { self.0.check_key_fields() }
    fn not_recorded_fields(&self) -> u64 { self.0.not_recorded_fields() }
    fn visit_nested_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) { self.0.visit_nested_field_gaps(visit); }
    fn recorded_fields_bytes(&self) -> io::Result<usize> { self.0.recorded_fields_bytes() }
    fn delta(&self, previous: Option<&Self>) -> io::Result<Option<DeltaRow>> { self.0.delta(previous.map(|row| &row.0)) }
    fn encode_change_reserved(&self, previous: Option<&Self>, key: u128, output: &mut Vec<u8>) -> io::Result<bool> {
        self.0.encode_change_reserved(previous.map(|row| &row.0), key, output)
    }
    fn encode_fields_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> { self.0.encode_fields_reserved(output) }
    fn decode_fields(key: u128, input: &mut Decoder<'_>) -> io::Result<Self> { CountryAppearanceRow::decode_fields(key, input).map(|row| Self(row, false)) }
    fn apply_delta(delta: &DeltaRow, previous: Option<Self>) -> io::Result<Self> { CountryAppearanceRow::apply_delta(delta, previous.map(|row| row.0)).map(|row| Self(row, false)) }
    fn has_display_steps(&self) -> bool { self.0.has_display_steps() }
    fn round_to_display(&mut self) { self.0.round_to_display(); }
    fn from_wire<'a>(key: u128, wire: &mut dyn FnMut(usize) -> io::Result<Field<'a>>) -> io::Result<Self> {
        CountryAppearanceRow::from_wire(key, wire).map(|row| Self(row, false))
    }
    fn visit_wire(&self, visit: &mut dyn FnMut(usize, Field<'_>) -> io::Result<()>) -> io::Result<()> {
        self.0.visit_wire(&mut |index, field| {
            let field = if self.1 && index == column::<CountryAppearanceRow>("original_tag") {
                Field::Opaque(Opaque::Encoded(&[2], replay_data::FORMAT_VERSION))
            } else {
                field
            };
            visit(index, field)
        })
    }
}

fn snapshot<R>(rows: &[R]) -> Input<'_, R> {
    Input::Snapshot { rows, partial: None, read: true, sample: Sample::FromReadBit }
}

#[test]
fn one_malformed_original_tag_preserves_the_other_countrys_tag_in_both_readers() {
    let hour = 100;
    let branch = BranchStart { id: 1, session: 1, loaded_hour: None, first_hour: hour, parent: Parentage::Unknown, reason: BranchReason::SessionStart };
    let country_ids = [(9001, 7, b"ONE"), (9002, 8, b"TWO")];
    let countries = country_ids.map(|(id, canonical_slot, tag)| CountryRow {
        id, canonical_slot, tag: tag.to_vec(), ..Default::default()
    });
    let links = country_ids.map(|(id, canonical_slot, _)| CountryLinkRow { id, canonical_slot });
    let appearances = [(9001, true), (9002, false)].map(|(country, malformed)| {
        Appearance(CountryAppearanceRow {
            country,
            original_tag: Recorded::Value(Some(b"ORI".to_vec())),
            original_tag_reference: Recorded::Value(Some(1)),
            ..Default::default()
        }, malformed)
    });
    let writers = recorder_derive::writers_within(&[], &[]);
    let replay = Replay::from_synthetic(writers, CAMPAIGN.into(), session(1), branch, hour, |frame, hour| {
        frame.table(TableId::Clock, snapshot(&[ClockRow { hour, ..Default::default() }]))?;
        frame.table(TableId::Campaign, snapshot(&[CampaignRow { id: CAMPAIGN.into() }]))?;
        frame.table(TableId::Countries, snapshot(&countries))?;
        frame.table(TableId::CountryLinks, snapshot(&links))?;
        frame.table(TableId::CountryAppearance, snapshot(&appearances))
    }).unwrap().into_timelines(&Default::default()).unwrap();
    let source = Source::from_replay(dashboard_base::SourceKey { source_id: "malformed-original-tag".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(replay), &false.into()).unwrap();
    let query = dashboard_base::queries::query(&source, 1).unwrap();
    let mut frames = at_hour::Frames::new(&query).unwrap();
    let decoded = read_hour(&query, &mut frames, hour, &[], &[]).unwrap();
    let places = Places::new(&decoded);
    let run = query.timeline.stretches[0].run;
    let history = PlaceHistory::load(query.replay.recording(), run, hour, hour, Keys::All, Keys::These(&[]), false, &false.into()).unwrap();
    for (reader, [bad, good]) in [
        ("frame", [places.tag(7), places.tag(8)]),
        ("history", [history.tag_at(7, hour), history.tag_at(8, hour)]),
    ] {
        assert_eq!(bad.unwrap_err(), "original tag of country appearance not recorded", "{reader}");
        assert_eq!(good.as_deref(), Ok("ORI"), "{reader}");
    }
}
