use crate::land_events::{LIVE, Observer};
use crate::land_events::Drain;
use collector_primitives::table_capture::{grow_capacity, grow_vec};
use engine_allocation as allocation;
use engine_native_memory::walk::Outcome;
use replay_data::{
    rows::DestroyedDivisionsRow, rows::LandCombatResultsRow, rows::LandEquipmentLoss,
    rows::LandEventCoverageRow, rows::LandResultSide, rows::LandSideLosses, rows::TableId,
    rows::TableRows,
};
use replay_frame::frame::TableView;
pub const N: usize = 3;
pub const TABLES: [TableId; N] = [
    TableId::LandCombatResults,
    TableId::DestroyedDivisions,
    TableId::LandEventCoverage,
];
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Capacities {
    pub words: usize,
}

collector_primitives::capacity_max!(Capacities { words });
pub struct Captured {
    words: Vec<u64>,
    drained: Option<Drain>,
}
impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }
    /// # Safety
    /// Caller must own the active session's capture or publication failure path when invalidating
    /// live land-event observation.
    pub unsafe fn fail(&mut self, hour: i32, reason: &'static str) {
        // SAFETY: Caller owns the capture or publication failure path for land events.
        self.drained = Some(unsafe { crate::land_events::lost_drain(hour, reason) });
    }
    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            words: allocation::owned_vec(allocation::default_vec(c.words)),
            drained: None,
        }
    }
    pub fn reset(&mut self) {
        self.drained = None;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.words.capacity() * size_of::<u64>()
    }
    /// # Safety
    /// Caller holds exclusive `&mut` access to this frame while land-event storage stays live for
    /// the active session (or an isolated test) and teardown is excluded. Different frames may grow
    /// concurrently across spool and sizing threads because `land_events::prepare` locks `LOG_SWAP`.
    pub unsafe fn grow(&mut self, c: &mut Capacities) {
        self.grow_from(&LIVE, c);
    }

    pub(crate) fn grow_from(&mut self, observer: &Observer, c: &mut Capacities) {
        observer.prepare();
        c.words = c.words.max(observer.capacity());
        if let Some(d) = self.drained {
            grow_capacity(&mut c.words, d.required);
        }
        grow_vec(&mut self.words, c.words);
    }
    /// # Safety
    /// Only the admitted hour's sole observer boundary (or an isolated test owning land-event
    /// observation) may drain the shared land-event log and install offered storage while storage
    /// remains live and teardown is excluded.
    pub unsafe fn drain(&mut self, hour: i32, invalidating: bool) -> u64 {
        self.drain_from(&LIVE, hour, invalidating)
    }

    pub(crate) fn drain_from(&mut self, observer: &Observer, hour: i32, invalidating: bool) -> u64 {
        let start = collector_primitives::clock::qpc();
        self.drained = Some(observer.drain(&mut self.words, hour, invalidating));
        observer.install_offered();
        collector_primitives::clock::qpc()
            .saturating_sub(start)
            .max(0) as u64
    }

    /// # Safety
    /// Caller must hold hourly capture admission for the active session (or an isolated test
    /// owning land-event observation) so global casualty-read loss accounting belongs to this pass.
    pub unsafe fn lost_casualty_reads(&mut self, count: u64) {
        self.lost_casualty_reads_from(&LIVE, count);
    }

    pub(crate) fn lost_casualty_reads_from(&mut self, observer: &Observer, count: u64) {
        if count != 0 {
            let lost = observer.lost_casualty_reads(count);
            if let Some(drain) = &mut self.drained {
                drain.lost = drain.lost.max(lost);
            }
        }
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        let coverage = if self.drained.is_some() {
            Outcome::Rows { len: 1 }
        } else {
            Outcome::Unavailable {
                reason: "land event drain unavailable",
            }
        };
        let counts = self
            .drained
            .map_or(Err("land event drain unavailable"), |d| {
                if d.complete {
                    counts(&self.words[..d.words])
                } else {
                    Err(d.reason)
                }
            });
        match counts {
            Ok([a, b]) => [Outcome::Rows { len: a }, Outcome::Rows { len: b }, coverage],
            Err(reason) => [
                Outcome::Unavailable { reason },
                Outcome::Unavailable { reason },
                coverage,
            ],
        }
    }
    pub fn tables(&self) -> [TableView; N] {
        let Some(d) = self.drained else {
            return std::array::from_fn(|_| TableView::Unavailable {
                reason: "land event drain unavailable".into(),
            });
        };
        let mut coverage = LandEventCoverageRow {
            id: 0,
            epoch: d.epoch,
            from_hour: d.from_hour,
            to_hour: d.to_hour,
            complete: d.complete,
            lost_events: d.lost,
        };
        let result = if d.complete {
            decode(&self.words[..d.words], d.first).and_then(|(results, destroyed)| {
                if results
                    .iter()
                    .any(|r| r.end_hour < d.from_hour || r.end_hour > d.to_hour)
                    || destroyed
                        .iter()
                        .any(|r| r.hour < d.from_hour || r.hour > d.to_hour)
                {
                    Err("land event timestamp outside observed interval")
                } else {
                    Ok((results, destroyed))
                }
            })
        } else {
            Err(d.reason)
        };
        match result {
            Ok((results, destroyed)) => [
                TableView::Complete(TableRows::LandCombatResults(results)),
                TableView::Complete(TableRows::DestroyedDivisions(destroyed)),
                coverage_table(coverage),
            ],
            Err(reason) => {
                coverage.complete = false;
                [
                    TableView::Unavailable {
                        reason: reason.into(),
                    },
                    TableView::Unavailable {
                        reason: reason.into(),
                    },
                    coverage_table(coverage),
                ]
            }
        }
    }
}
fn coverage_table(row: LandEventCoverageRow) -> TableView {
    match allocation::collect_vec([row]) {
        Ok(rows) => TableView::Complete(TableRows::LandEventCoverage(rows)),
        Err(_) => {
            collector_projection::table_conversion::unavailable("collector allocation failed")
        }
    }
}
fn counts(words: &[u64]) -> Result<[usize; 2], &'static str> {
    let mut counts = [0; 2];
    let mut at = 0;
    while at < words.len() {
        let size = usize::try_from(words[at]).map_err(|_| "land event size overflow")?;
        let end = at
            .checked_add(size)
            .filter(|end| size >= 4 && *end <= words.len())
            .ok_or("land event framing invalid")?;
        match words[at + 1] {
            0 => counts[0] += 1,
            1 | 7 => counts[1] += 1,
            2..=6 | 8 | 9 => {}
            _ => return Err("land event kind unsupported"),
        };
        at = end;
    }
    Ok(counts)
}
struct Words<'a> {
    words: &'a [u64],
    at: usize,
}
impl Words<'_> {
    fn word(&mut self) -> Result<u64, &'static str> {
        let word = *self.words.get(self.at).ok_or("land event record short")?;
        self.at += 1;
        Ok(word)
    }
    fn tags(&mut self) -> Result<Vec<u32>, &'static str> {
        let count = usize::try_from(self.word()?).map_err(|_| "land tag count overflow")?;
        if count > self.words.len() - self.at {
            return Err("land tag list short");
        }
        let mut tags = allocation::vec(count).map_err(|_| "collector allocation failed")?;
        for _ in 0..count {
            tags.push(self.word()? as u32);
        }
        Ok(tags)
    }
    fn null_tag(&mut self) -> Result<Vec<u8>, &'static str> {
        let len = self.word()?;
        let len = usize::try_from(len).map_err(|_| "null tag length overflow")?;
        if len.div_ceil(8) > self.words.len() - self.at {
            return Err("null tag payload short");
        }
        let mut bytes = allocation::vec(len).map_err(|_| "collector allocation failed")?;
        for _ in 0..len.div_ceil(8) {
            let word = self.word()?.to_le_bytes();
            let n = (len - bytes.len()).min(8);
            bytes.extend_from_slice(&word[..n]);
        }
        Ok(bytes)
    }
    fn losses(&mut self) -> Result<LandSideLosses, &'static str> {
        let manpower = self.word()? as i32;
        let count = usize::try_from(self.word()?).map_err(|_| "land loss count overflow")?;
        if count > (self.words.len() - self.at) / 2 {
            return Err("land loss pool short");
        }
        let mut equipment = allocation::vec(count).map_err(|_| "collector allocation failed")?;
        for _ in 0..count {
            equipment.push(LandEquipmentLoss {
                variant: self.word()?,
                amount_raw: self.word()? as i64,
            });
        }
        Ok(LandSideLosses {
            manpower,
            equipment,
        })
    }
    fn side(&mut self) -> Result<LandResultSide, &'static str> {
        let outcome_flags = self.word()? as u8;
        let progress = self.word()? as i64;
        let countries = self.tags()?;
        let secondary_countries = self.tags()?;
        let count = usize::try_from(self.word()?).map_err(|_| "land commander count overflow")?;
        if count > (self.words.len() - self.at) / 3 {
            return Err("land commander candidates short");
        }
        let mut commander = 0;
        let mut selected_skill = 0;
        let mut selected_category = 0;
        for _ in 0..count {
            let handle = self.word()?;
            let skill = self.word()? as i32;
            let category = self.word()? as i32;
            if handle != 0
                && (commander == 0
                    || skill > selected_skill
                    || (skill == selected_skill && selected_category == 0 && category == 1))
            {
                commander = handle;
                selected_skill = skill;
                selected_category = category;
            }
        }
        Ok(LandResultSide {
            countries,
            secondary_countries,
            commander,
            outcome_flags,
            progress,
        })
    }
}
fn decode(
    words: &[u64],
    first: u64,
) -> Result<(Vec<LandCombatResultsRow>, Vec<DestroyedDivisionsRow>), &'static str> {
    let mut at = 0;
    let mut results = Vec::new();
    let mut destroyed = Vec::new();
    let mut verdicts = std::collections::HashMap::new();
    let mut participants = std::collections::HashMap::new();
    let mut commanders = std::collections::HashMap::new();
    let mut histories = std::collections::HashMap::new();
    let mut losses = std::collections::HashMap::new();
    let mut observers: std::collections::HashMap<
        u64,
        Vec<replay_data::rows::LandTerminalObserver>,
    > = std::collections::HashMap::new();
    let mut destructions = std::collections::HashMap::new();
    while at < words.len() {
        let size = usize::try_from(words[at]).map_err(|_| "land event length overflow")?;
        if size < 4 {
            return Err("land event record invalid");
        }
        let end = at
            .checked_add(size)
            .filter(|end| *end <= words.len())
            .ok_or("land event length exceeds drain")?;
        let mut reader = Words {
            words: &words[at + 1..end],
            at: 0,
        };
        let kind = reader.word()?;
        let id = first + at as u64;
        match kind {
            0 | 3 => {
                let combat = reader.word()?;
                let province = reader.word()? as u32;
                let day = reader.word()? as i32;
                let duration = reader.word()? as i32;
                let end_hour = reader.word()? as i32;
                let winners = reader.word()? as u8;
                let border_war = reader.word()? != 0;
                let packed = reader.word()?;
                let sides = [reader.side()?, reader.side()?];
                let border_end_flags = [packed as u8, (packed >> 8) as u8, (packed >> 16) as u8];
                let border_terminating = [(packed >> 24) as u8, (packed >> 32) as u8];
                if !border_war
                    && winners
                        != engine_land_inputs::land_events::winners(
                            sides.each_ref().map(|s| s.outcome_flags),
                            sides.each_ref().map(|s| s.progress),
                        )
                {
                    return Err("land result verdict differs from raw inputs");
                }
                let row = LandCombatResultsRow {
                    id,
                    combat,
                    province,
                    day,
                    duration,
                    end_hour,
                    winners,
                    border_war,
                    border_end_flags,
                    border_terminating,
                    history_countries: Default::default(),
                    on_action_winners: None,
                    history_tags: Default::default(),
                    sides,
                    losses: Default::default(),
                    observers: Default::default(),
                };
                if kind == 3 {
                    if insert(&mut participants, combat, row.sides)? {
                        return Err("duplicate termination participants");
                    }
                } else {
                    allocation::reserve_push_batch(&mut results, 1)
                        .map_err(|_| "collector allocation failed")?;
                    results.push(row);
                }
            }
            1 => {
                let division = reader.word()?;
                let owner = reader.word()? as u32;
                let killer_country = reader.word()? as u32;
                let killer_division = reader.word()?;
                let province = reader.word()? as u32;
                let destination = reader.word()? as u32;
                let hour = reader.word()? as i32;
                let count = usize::try_from(reader.word()?)
                    .map_err(|_| "destroyed combat count overflow")?;
                if count > reader.words.len() - reader.at {
                    return Err("destroyed combat list short");
                }
                let mut combats =
                    allocation::vec(count).map_err(|_| "collector allocation failed")?;
                for _ in 0..count {
                    combats.push(reader.word()?);
                }
                allocation::reserve_push_batch(&mut destroyed, 1)
                    .map_err(|_| "collector allocation failed")?;
                allocation::reserve_map(&mut destructions, 1)
                    .map_err(|_| "collector allocation failed")?;
                destructions.insert(id, destroyed.len());
                destroyed.push(DestroyedDivisionsRow {
                    id,
                    division,
                    owner,
                    killer_country,
                    killer_division: Some(killer_division),
                    province,
                    destination: Some(destination),
                    combats,
                    hour,
                    triggering_combat: None,
                    combat_province: None,
                });
            }
            2 => {
                let combat = reader.word()?;
                let packed = reader.word()?;
                let inputs = [
                    reader.word()?,
                    reader.word()?,
                    reader.word()?,
                    reader.word()?,
                ];
                if insert(&mut verdicts, combat, (packed, inputs))? {
                    return Err("duplicate border verdict");
                }
            }
            4 => {
                let combat = reader.word()?;
                let leaders = [reader.word()?, reader.word()?];
                let winner = reader.word()? as u8;
                if winner != 1 && winner != 2 {
                    return Err("land on-action winner invalid");
                }
                if insert(&mut commanders, combat, (leaders, winner))? {
                    return Err("duplicate termination commanders");
                }
            }
            5 => {
                let combat = reader.word()?;
                let countries = [reader.word()? as u32, reader.word()? as u32];
                let tags = [reader.null_tag()?, reader.null_tag()?];
                if insert(&mut histories, combat, (countries, tags))? {
                    return Err("duplicate final history projection");
                }
            }
            6 => {
                let entry = reader.word()?;
                let killer = reader.word()?;
                let index = destructions
                    .remove(&entry)
                    .ok_or("killer observation has no destruction entry")?;
                destroyed[index].killer_division = Some(killer);
            }
            7 => {
                let hour = reader.word()? as i32;
                let division = reader.word()?;
                let owner = reader.word()? as u32;
                let killer_country = reader.word()? as u32;
                let province = reader.word()? as u32;
                let combat_province = reader.word()? as u32;
                let triggering_combat = reader.word()?;
                let count = usize::try_from(reader.word()?)
                    .map_err(|_| "destroyed combat count overflow")?;
                if count > reader.words.len() - reader.at {
                    return Err("destroyed combat list short");
                }
                let mut combats =
                    allocation::vec(count).map_err(|_| "collector allocation failed")?;
                for _ in 0..count {
                    combats.push(reader.word()?);
                }
                allocation::reserve_push_batch(&mut destroyed, 1)
                    .map_err(|_| "collector allocation failed")?;
                destroyed.push(DestroyedDivisionsRow {
                    id,
                    division,
                    owner,
                    killer_country,
                    killer_division: None,
                    province,
                    destination: None,
                    combats,
                    hour,
                    triggering_combat: Some(triggering_combat),
                    combat_province: Some(combat_province),
                });
            }
            8 => {
                let combat = reader.word()?;
                let sides = [reader.losses()?, reader.losses()?];
                if insert(&mut losses, combat, sides)? {
                    return Err("duplicate terminal land losses");
                }
            }
            9 => {
                let combat = reader.word()?;
                let side = reader.word()?;
                let kind = reader.word()?;
                if side > 1 || kind > u8::MAX as u64 {
                    return Err("terminal observer identity invalid");
                }
                let item = reader.word()?;
                let values = [
                    reader.word()?,
                    reader.word()?,
                    reader.word()?,
                    reader.word()?,
                    reader.word()?,
                ];
                if !observers.contains_key(&combat) {
                    allocation::reserve_map(&mut observers, 1)
                        .map_err(|_| "collector allocation failed")?;
                }
                let rows = observers.entry(combat).or_default();
                allocation::reserve_push_batch(rows, 1)
                    .map_err(|_| "collector allocation failed")?;
                rows.push(replay_data::rows::LandTerminalObserver {
                    attacker: side == 0,
                    kind: kind as u8,
                    item,
                    values,
                });
            }
            _ => return Err("land event kind unsupported"),
        }
        if reader.at != reader.words.len() {
            return Err("land event trailing words");
        }
        at = end;
    }
    for result in results.iter_mut().filter(|r| r.border_war) {
        let (packed, inputs) = verdicts
            .remove(&result.combat)
            .ok_or("border verdict missing")?;
        let flags = [packed as u8, (packed >> 8) as u8, (packed >> 16) as u8];
        if flags != result.border_end_flags {
            return Err("border flags differ");
        }
        result.border_terminating = [(packed >> 24) as u8, (packed >> 32) as u8];
        for n in 0..2 {
            result.sides[n].outcome_flags = inputs[n * 2] as u8;
            result.sides[n].progress = inputs[n * 2 + 1] as i64;
        }
        result.winners =
            engine_land_inputs::land_events::border_winners(flags, result.border_terminating)?;
    }
    for result in &mut results {
        let (countries, tags) = histories
            .remove(&result.combat)
            .ok_or("final history projection missing")?;
        result.history_countries = countries;
        result.history_tags = tags;
        result.losses = losses
            .remove(&result.combat)
            .ok_or("terminal land losses missing")?;
        result.observers = observers
            .remove(&result.combat)
            .ok_or("terminal observers missing")?;
        for attacker in [true, false] {
            if result
                .observers
                .iter()
                .filter(|r| {
                    r.attacker == attacker
                        && r.kind == engine_land_inputs::combat_log_events::OBSERVER_STATE as u8
                })
                .count()
                != 1
            {
                return Err("terminal observer side state missing or repeated");
            }
        }
        if let Some(sides) = participants.remove(&result.combat) {
            let (leaders, winner) = commanders
                .remove(&result.combat)
                .ok_or("termination commanders missing")?;
            result.on_action_winners = Some(winner);
            for (n, side) in sides.into_iter().enumerate() {
                result.sides[n].countries = side.countries;
                result.sides[n].secondary_countries = side.secondary_countries;
                result.sides[n].commander = leaders[n];
            }
        }
    }
    if !participants.is_empty() || !commanders.is_empty() {
        return Err("termination snapshot has no final result");
    }
    if !verdicts.is_empty() {
        return Err("border verdict has no entry snapshot");
    }
    if !losses.is_empty() {
        return Err("terminal land losses have no final result");
    }
    if !observers.is_empty() {
        return Err("terminal observers have no final result");
    }
    if !histories.is_empty() {
        return Err("history projection has no final result");
    }
    if !destructions.is_empty() {
        return Err("destruction killer observation missing");
    }
    Ok((results, destroyed))
}
fn insert<T>(
    entries: &mut std::collections::HashMap<u64, T>,
    key: u64,
    value: T,
) -> Result<bool, &'static str> {
    if entries.contains_key(&key) {
        return Ok(true);
    }
    allocation::reserve_map(entries, 1).map_err(|_| "collector allocation failed")?;
    entries.insert(key, value);
    Ok(false)
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::land_events::test_support;

    fn complete_coverage(tables: &[TableView; N]) -> &LandEventCoverageRow {
        let TableView::Complete(TableRows::LandEventCoverage(rows)) = &tables[2] else {
            panic!("land coverage unavailable")
        };
        &rows[0]
    }
    #[test]
    fn land_event_buffers_grow_past_the_former_cap_in_the_drain() {
        let fixture = test_support::Fixture::new(0);
        let mut captured = Captured::with_capacities(&Capacities::default());
        captured.drain_from(&fixture.observer, 10, true);
        captured.drain_from(&fixture.observer, 11, false);
        let payload = vec![0; (1 << 20) + 17];
        let lost = fixture.lost();
        fixture.append_record(&payload);
        captured.drain_from(&fixture.observer, 12, false);
        let drained = captured.drained.unwrap();
        assert!(drained.complete, "{}", drained.reason);
        assert_eq!(drained.words, payload.len() + 1);
        assert!(captured.words.len() > payload.len());
        assert_eq!(fixture.lost(), lost);
        fixture.stop();
    }
    fn encode_land(
        encoder: &mut replay_frame::frame::FrameEncoder,
        hour: i32,
        land: [TableView; N],
        forced: bool,
    ) -> replay_frame::frame::Frame {
        let mut tables = std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture omitted".into(),
            }
            .into()
        });
        for (id, view) in TABLES.into_iter().zip(land) {
            tables[id.index()] = view.into();
        }
        encoder
            .encode(
                replay_frame::frame::DecodedFrame {
                    branch: 1,
                    hour,
                    tables,
                    metrics: Default::default(),
                },
                forced,
            )
            .unwrap()
    }
    #[test]
    fn refresh_full_read_keeps_actual_land_drain_epoch_and_events() {
        let fixture = test_support::Fixture::new(1024);
        let mut captured = Captured::with_capacities(&Capacities { words: 1024 });
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let mut decoder = replay_frame::frame::FrameDecoder::new();
        captured.drain_from(&fixture.observer, 10, true);
        let first = captured.tables();
        let epoch = complete_coverage(&first).epoch;
        assert!(!complete_coverage(&first).complete);
        decoder
            .advance(&encode_land(&mut encoder, 10, first, false))
            .unwrap();
        fixture.append_record(&[7, 11, 17, 4999, 1500, 9, 10, 41, 2, 42, 42]);
        captured.reset();
        captured.drain_from(&fixture.observer, 11, false);
        let refreshed = captured.tables();
        let coverage = complete_coverage(&refreshed);
        assert_eq!(coverage.epoch, epoch);
        assert!(coverage.complete);
        assert_eq!((coverage.from_hour, coverage.to_hour), (10, 11));
        assert_eq!(
            captured.outcomes(),
            [
                Outcome::Rows { len: 0 },
                Outcome::Rows { len: 1 },
                Outcome::Rows { len: 1 },
            ]
        );
        let TableView::Complete(TableRows::DestroyedDivisions(rows)) = &refreshed[1] else {
            panic!()
        };
        assert_eq!(rows[0].killer_division, None);
        assert_eq!(rows[0].destination, None);
        assert_eq!(rows[0].triggering_combat, Some(41));
        assert_eq!(rows[0].combat_province, Some(10));
        assert_eq!(rows[0].province, 9);
        assert_eq!(rows[0].combats, [42, 42]);
        let expected = refreshed[1].clone();
        let decoded = decoder
            .advance(&encode_land(&mut encoder, 11, refreshed, true))
            .unwrap();
        assert_eq!(
            &*decoded.tables[TableId::DestroyedDivisions.index()],
            &expected
        );
        fixture.stop();
    }
    #[test]
    fn actual_land_drain_load_and_gap_invalidate_but_next_hour_recovers() {
        let fixture = test_support::Fixture::new(1024);
        let mut captured = Captured::with_capacities(&Capacities { words: 1024 });
        captured.drain_from(&fixture.observer, 10, true);
        captured.drain_from(&fixture.observer, 11, false);
        let initial = complete_coverage(&captured.tables()).epoch;
        for (hour, invalidating) in [(20, true), (30, false)] {
            fixture.append_record(&[7, hour as u64, 17, 1, 2, 9, 10, 41, 0]);
            captured.reset();
            captured.drain_from(&fixture.observer, hour, invalidating);
            let unknown = captured.tables();
            let coverage = complete_coverage(&unknown);
            assert!(!coverage.complete);
            assert!(coverage.epoch > initial);
            assert!(matches!(unknown[0], TableView::Unavailable { .. }));
            assert!(matches!(unknown[1], TableView::Unavailable { .. }));
            let epoch = coverage.epoch;
            captured.reset();
            captured.drain_from(&fixture.observer, hour + 1, false);
            let recovered = captured.tables();
            let coverage = complete_coverage(&recovered);
            assert!(coverage.complete);
            assert_eq!(coverage.epoch, epoch);
            assert_eq!((coverage.from_hour, coverage.to_hour), (hour, hour + 1));
            assert_eq!(
                recovered[1],
                TableView::Complete(TableRows::DestroyedDivisions(vec![]))
            );
        }
        fixture.stop();
    }
    #[test]
    fn unpublished_actual_land_drain_cannot_be_covered_by_the_next_frame() {
        let fixture = test_support::Fixture::new(1024);
        let mut captured = Captured::with_capacities(&Capacities { words: 1024 });
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let mut decoder = replay_frame::frame::FrameDecoder::new();
        captured.drain_from(&fixture.observer, 10, true);
        decoder
            .advance(&encode_land(&mut encoder, 10, captured.tables(), false))
            .unwrap();
        captured.drain_from(&fixture.observer, 11, false);
        decoder
            .advance(&encode_land(&mut encoder, 11, captured.tables(), false))
            .unwrap();
        fixture.append_record(&[7, 12, 17, 1, 2, 9, 10, 41, 0]);
        captured.drain_from(&fixture.observer, 12, false);
        assert_eq!(captured.outcomes()[1], Outcome::Rows { len: 1 });
        let dropped_epoch = complete_coverage(&captured.tables()).epoch;
        captured.reset();
        captured.drain_from(&fixture.observer, 13, false);
        let next = captured.tables();
        let coverage = complete_coverage(&next);
        assert!(coverage.complete);
        assert_eq!(coverage.epoch, dropped_epoch);
        assert_eq!((coverage.from_hour, coverage.to_hour), (12, 13));
        let decoded = decoder
            .advance(&encode_land(&mut encoder, 13, next, true))
            .unwrap();
        let TableView::Complete(TableRows::LandEventCoverage(rows)) =
            &*decoded.tables[TableId::LandEventCoverage.index()]
        else {
            panic!()
        };
        assert_eq!((rows[0].from_hour, rows[0].to_hour), (12, 13));
        assert!(
            rows[0].from_hour > 11,
            "published hour 11 cannot certify the missing span"
        );
        assert_eq!(
            &*decoded.tables[TableId::DestroyedDivisions.index()],
            &TableView::Complete(TableRows::DestroyedDivisions(vec![]))
        );
        fixture.stop();
    }
    #[test]
    fn wipe_decode_refuses_short_or_trailing_records_and_preserves_native_context() {
        let words = [12, 7, 80, 17, 4999, 1500, 9, 10, 41, 2, 42, 42];
        let (results, destroyed) = decode(&words, 77).unwrap();
        assert!(results.is_empty());
        assert_eq!(destroyed[0].id, 77);
        assert_eq!(destroyed[0].killer_division, None);
        assert_eq!(destroyed[0].destination, None);
        assert_eq!(destroyed[0].triggering_combat, Some(41));
        assert_eq!(destroyed[0].combat_province, Some(10));
        assert_eq!(destroyed[0].combats, [42, 42]);
        for len in 1..words.len() {
            assert!(decode(&words[..len], 77).is_err());
        }
        let mut trailing = words.to_vec();
        trailing.push(42);
        trailing[0] += 1;
        assert!(decode(&trailing, 77).is_err());
    }
    #[test]
    fn combat_log_terminal_casualties_survive_an_end_between_reads_and_require_complete_pairing() {
        let mut words = result(0, false, false);
        let start = words.len();
        let mut history = history(41, [0, 0]);
        history.truncate(9);
        words.extend(history);
        words.extend([9, 8, 41, 83, 1, 7 << 32 | 70, 125_001, 91, 0]);
        words.extend(terminal_observers(41));
        let (ended, _) = decode(&words, 0).unwrap();
        assert_eq!(ended[0].losses[0].manpower, 83);
        assert_eq!(ended[0].losses[0].equipment[0].amount_raw, 125_001);
        assert_eq!(ended[0].losses[1].manpower, 91);
        assert!(ended[0].losses[1].equipment.is_empty());
        assert_eq!(ended[0].observers.len(), 2);
        assert_eq!(ended[0].observers[0].values[0..2], [123, 456]);
        assert!(ended[0].observers[0].attacker);
        assert!(!ended[0].observers[1].attacker);
        assert!(
            decode(&words[..words.len() - 11], 0).is_err(),
            "a missing terminal side must refuse"
        );
        assert!(
            decode(&words[..start + 9], 0).is_err(),
            "missing terminal payload is unknown"
        );
        let terminal = words[start + 9..].to_vec();
        words.extend_from_slice(&terminal);
        assert!(decode(&words, 0).is_err(), "duplicate terminal payload");
        assert!(decode(&terminal, 0).is_err(), "orphan terminal payload");
    }
    #[test]
    fn terminal_losses_past_the_log_are_kept_and_only_failed_storage_counts_a_gap() {
        let fixture = test_support::Fixture::new(8);
        let mut c = Captured::with_capacities(&Capacities { words: 32 });
        c.drain_from(&fixture.observer, 10, true);
        c.drain_from(&fixture.observer, 11, false);
        let terminal = [
            8,
            41,
            73,
            1,
            1 << 32 | 70,
            234_567,
            91,
            1,
            2 << 32 | 70,
            987_654,
        ];
        let lost = fixture.lost();
        fixture.append_record(&terminal);
        c.drain_from(&fixture.observer, 12, false);
        let drained = c.drained.unwrap();
        assert!(drained.complete, "{}", drained.reason);
        assert_eq!(drained.words, terminal.len() + 1);
        assert_eq!(c.words[0], terminal.len() as u64 + 1);
        assert_eq!(c.words[1..drained.words], terminal);
        assert_eq!(fixture.lost(), lost);
        collector_test_support::with_allocations_denied(|| fixture.append_record(&terminal));
        c.drain_from(&fixture.observer, 13, false);
        assert_eq!(
            c.drained.unwrap().reason,
            "land event storage allocation failed"
        );
        let tables = c.tables();
        let coverage = complete_coverage(&tables);
        assert!(!coverage.complete);
        assert!(coverage.lost_events > 0);
        assert!(matches!(tables[0], TableView::Unavailable { .. }));
        c.drain_from(&fixture.observer, 14, false);
        assert!(complete_coverage(&c.tables()).complete);
        fixture.stop();
    }
    fn result(kind: u64, border: bool, tags: bool) -> Vec<u64> {
        let mut out = vec![
            0,
            kind,
            41,
            9,
            2,
            24,
            80,
            if border { 255 } else { 1 },
            u64::from(border),
            0,
        ];
        for n in 0..2 {
            out.extend([if n == 0 { 2 } else { 0 }, 17]);
            if tags {
                out.extend([1, 101 + n, 1, 201 + n]);
            } else {
                out.extend([0, 0]);
            }
            out.push(0);
        }
        out[0] = out.len() as u64;
        out
    }
    fn history(combat: u64, countries: [u32; 2]) -> Vec<u64> {
        let mut out = vec![
            9,
            5,
            combat,
            u64::from(countries[0]),
            u64::from(countries[1]),
            3,
            u64::from_le_bytes([b'?', b'!', b'~', 0, 0, 0, 0, 0]),
            3,
            u64::from_le_bytes([b'?', b'!', b'~', 0, 0, 0, 0, 0]),
        ];
        out.extend([7, 8, combat, 0, 0, 0, 0]);
        out.extend(terminal_observers(combat));
        out
    }
    fn terminal_observers(combat: u64) -> Vec<u64> {
        [0, 1]
            .into_iter()
            .flat_map(|side| {
                [
                    11,
                    9,
                    combat,
                    side,
                    engine_land_inputs::combat_log_events::OBSERVER_STATE,
                    0,
                    123,
                    456,
                    0,
                    0,
                    0,
                ]
            })
            .collect()
    }
    #[test]
    fn border_draw_and_pre_retreat_participants_pair_with_actual_commanders_and_final_inputs() {
        let mut words = result(3, true, true);
        words.extend([6, 4, 41, 81, 82, 1]);
        words.extend(result(0, true, false));
        let at = words.len();
        words.extend([
            8,
            2,
            41,
            1 << 24 | 1 << 32,
            0,
            i64::MIN as u64,
            2,
            i64::MAX as u64,
        ]);
        words.extend(history(41, [0, 0]));
        let (rows, dead) = decode(&words, 0).unwrap();
        assert!(dead.is_empty());
        let row = &rows[0];
        assert_eq!(row.winners, 0);
        assert_eq!(row.on_action_winners, Some(1));
        assert_eq!(row.sides[0].countries, [101]);
        assert_eq!(row.sides[1].secondary_countries, [202]);
        assert_eq!(row.sides[1].commander, 82);
        assert_eq!(row.sides[0].progress, i64::MIN);
        assert_eq!(row.sides[1].outcome_flags, 2);
        assert_eq!(row.history_countries, [0, 0]);
        assert_eq!(row.history_tags, [0; 2].map(|_| b"?!~".to_vec()));
        assert!(decode(&words[..at], 0).is_err());
        assert!(decode(&words[result(3, true, true).len() + 6..], 0).is_ok());
    }
    #[test]
    fn owned_decode_preserves_wide_signed_inputs_candidates_and_verdict() {
        let mut record = vec![0, 0, 0x8000_0001_0000_0041, 9, 1, 2, 80, 1, 0, 0];
        record.extend([
            2,
            i64::MIN as u64,
            2,
            0x8000_0001,
            17,
            0,
            3,
            11,
            2,
            0,
            12,
            2,
            1,
            13,
            1,
            1,
        ]);
        record.extend([0, i64::MAX as u64, 0, 0, 0]);
        record[0] = record.len() as u64;
        record.extend(history(0x8000_0001_0000_0041, [0, 0]));
        let (results, dead) = decode(&record, 7).unwrap();
        assert!(dead.is_empty());
        assert_eq!(results[0].id, 7);
        assert_eq!(results[0].sides[0].commander, 12);
        assert_eq!(results[0].sides[0].progress, i64::MIN);
        assert_eq!(results[0].winners, 1);
        for len in 0..record.len() {
            if len != 0 {
                assert!(decode(&record[..len], 0).is_err());
            }
        }
    }
    #[test]
    fn a_death_stamped_with_the_previous_read_lies_in_its_interval_and_an_older_one_refuses() {
        let words = vec![12, 7, 80, 17, 4999, 1500, 9, 10, 41, 2, 42, 42];
        let tables = |from_hour| {
            Captured {
                words: words.clone(),
                drained: Some(Drain {
                    epoch: 1,
                    first: 0,
                    words: words.len(),
                    required: words.len(),
                    from_hour,
                    to_hour: from_hour + 1,
                    complete: true,
                    lost: 0,
                    reason: "",
                }),
            }
            .tables()
        };
        assert!(
            matches!(&tables(80)[1], TableView::Complete(TableRows::DestroyedDivisions(rows)) if rows.len() == 1)
        );
        let older = tables(81);
        assert!(!complete_coverage(&older).complete);
        assert!(matches!(older[1], TableView::Unavailable { .. }));
    }
    #[test]
    fn unknown_interval_is_never_an_empty_event_table() {
        let c = Captured {
            words: vec![],
            drained: Some(Drain {
                epoch: 1,
                first: 0,
                words: 0,
                required: 0,
                from_hour: 9,
                to_hour: 10,
                complete: false,
                lost: 1,
                reason: "overflow",
            }),
        };
        let [a, b, coverage] = c.tables();
        assert!(matches!(a, TableView::Unavailable { .. }));
        assert!(matches!(b, TableView::Unavailable { .. }));
        let TableView::Complete(TableRows::LandEventCoverage(rows)) = coverage else {
            panic!()
        };
        assert!(!rows[0].complete);
    }
    #[test]
    fn repeated_and_nested_destruction_observations_preserve_each_killer_and_sibling_table() {
        let first = 77;
        let division = 0x8000_0001_0000_1269;
        let mut words = result(0, false, true);
        words.extend(history(41, [1, 2]));
        let outer = first + words.len() as u64;
        words.extend([12, 1, division, 1, 2, 0, 7, 8, 80, 2, 41, 41]);
        let inner = first + words.len() as u64;
        words.extend([10, 1, division, 3, 4, 0, 9, 10, 80, 0]);
        words.extend([4, 6, inner, 0x8000_0002_0000_0042]);
        words.extend([4, 6, outer, 0x8000_0003_0000_0043]);
        let repeated = first + words.len() as u64;
        words.extend([10, 1, division, 5, 6, 0, 11, 12, 80, 0]);
        let paired = words.len();
        assert_eq!(
            decode(&words, first).unwrap_err(),
            "destruction killer observation missing"
        );
        words.extend([4, 6, repeated, 0]);
        let (_, destroyed) = decode(&words, first).unwrap();
        assert_eq!(destroyed.len(), 3);
        assert_eq!(
            destroyed.iter().map(|row| row.id).collect::<Vec<_>>(),
            [outer, inner, repeated]
        );
        assert!(destroyed.iter().all(|row| row.division == division));
        assert_eq!(destroyed[0].killer_division, Some(0x8000_0003_0000_0043));
        assert_eq!(destroyed[1].killer_division, Some(0x8000_0002_0000_0042));
        assert_eq!(destroyed[2].killer_division, Some(0));
        assert_eq!(destroyed[0].combats, [41, 41]);
        for (row, owner, killer_country, province, destination) in [
            (&destroyed[0], 1, 2, 7, 8),
            (&destroyed[1], 3, 4, 9, 10),
            (&destroyed[2], 5, 6, 11, 12),
        ] {
            assert_eq!(row.owner, owner);
            assert_eq!(row.killer_country, killer_country);
            assert_eq!(row.province, province);
            assert_eq!(row.destination, Some(destination));
        }
        let captured = Captured {
            drained: Some(Drain {
                epoch: 1,
                first,
                words: words.len(),
                required: words.len(),
                from_hour: 79,
                to_hour: 80,
                complete: true,
                lost: 0,
                reason: "",
            }),
            words: words.clone(),
        };
        let tables = captured.tables();
        assert!(complete_coverage(&tables).complete);
        assert!(
            matches!(&tables[0], TableView::Complete(TableRows::LandCombatResults(rows)) if rows.len() == 1)
        );
        assert_eq!(
            tables[1],
            TableView::Complete(TableRows::DestroyedDivisions(destroyed))
        );
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let frame = encode_land(&mut encoder, 80, tables.clone(), true);
        let mut decoder = replay_frame::frame::FrameDecoder::new();
        let decoded = decoder.advance(&frame).unwrap();
        for (id, expected) in TABLES.into_iter().zip(&tables) {
            assert_eq!(&*decoded.tables[id.index()], expected);
        }
        words.extend_from_within(paired..);
        assert_eq!(
            decode(&words, first).unwrap_err(),
            "killer observation has no destruction entry"
        );
    }
    #[test]
    fn native_destruction_to_replay_roundtrip_recovers_without_stale_events() {
        #[derive(Default)]
        struct Memory(std::collections::BTreeMap<usize, u8>);
        impl engine_native_memory::Memory for Memory {
            fn copy(&self, p: usize, out: &mut [u8]) -> bool {
                for (n, b) in out.iter_mut().enumerate() {
                    let Some(v) = self.0.get(&(p + n)) else {
                        return false;
                    };
                    *b = *v;
                }
                true
            }
        }
        impl Memory {
            fn put(&mut self, p: usize, b: &[u8]) {
                for (n, v) in b.iter().enumerate() {
                    self.0.insert(p + n, *v);
                }
            }
            fn q(&mut self, p: usize, v: u64) {
                self.put(p, &v.to_le_bytes());
            }
            fn d(&mut self, p: usize, v: u32) {
                self.put(p, &v.to_le_bytes());
            }
            fn array(&mut self, p: usize, data: usize, count: u32) {
                self.q(p, data as u64);
                self.d(p + 8, count);
                self.d(p + 12, count);
            }
        }
        use engine_native::{carmy, ccountry, cgame_state, cland_combat, cprovince, cunit};
        let mut m = Memory::default();
        m.q(engine_native::cgame_state::SINGLETON_RVA.0, 0x1000);
        m.d(0x1000 + engine_native::cgame_state::CURRENT_HOUR, 80);
        m.array(0x1000 + cgame_state::COUNTRIES, 0x9000, 2);
        m.array(0x1000 + cgame_state::COUNTRY_LINKS, 0xa000, 2);
        m.q(0x9008, 0xb000);
        m.d(0xa004, 1);
        m.q(0xb000, ccountry::VTABLE_RVA.0 as u64);
        m.d(0xb000 + ccountry::ID, 1);
        m.q(0x2000, carmy::VTABLE_RVA.0 as u64);
        m.q(0x2000 + cunit::HANDLE, 0x8000_0001_0000_1269);
        m.d(0x2000 + cunit::ROSTER_OWNER, 1);
        m.d(0x2000 + cunit::LOGICAL_OWNER, 1);
        for (p, id) in [(0x3000, 7), (0x4000, 8)] {
            m.q(p, cprovince::VTABLE_RVA.0 as u64);
            m.d(p + cprovince::ID, id);
            m.d(p + cprovince::CONTROLLER, 1);
        }
        m.array(0x2000 + cunit::COMBAT_REFERENCE_DATA, 0x6000, 1);
        m.q(0x6000, 0x7000);
        m.q(0x7000, cland_combat::VTABLE_RVA.0 as u64);
        m.q(0x7000 + cland_combat::HANDLE, 0x8000_0003_0000_003e);
        let plan = engine_land_inputs::land_events::DestructionPlan::read(
            &m, 0, 0x3000, 0x4000, 0x2000, 0,
        )
        .unwrap();
        let mut words = vec![plan.words as u64];
        plan.copy(&m, 0, |word| words.push(word)).unwrap();
        let entry_len = words.len();
        assert!(decode(&words, 77).is_err());
        m.d(0x2000 + cunit::LOGICAL_OWNER, 0);
        m.d(0x4000 + cprovince::CONTROLLER, 0);
        words.extend([4, 6, 77, 0]);
        let (_, dead) = decode(&words, 77).unwrap();
        assert_eq!(dead[0].combats, [0x8000_0003_0000_003e]);
        assert_eq!(dead[0].killer_division, Some(0));
        assert_eq!(dead[0].owner, 1);
        assert_eq!(dead[0].killer_country, 1);
        assert_eq!(dead[0].destination, Some(8));
        assert_eq!(dead[0].triggering_combat, None);
        assert_eq!(dead[0].combat_province, None);
        assert!(decode(&words[entry_len..], 77).is_err());
        let c = Captured {
            words: words.clone(),
            drained: Some(Drain {
                epoch: 1,
                first: 77,
                words: words.len(),
                required: words.len(),
                from_hour: 79,
                to_hour: 80,
                complete: true,
                lost: 0,
                reason: "",
            }),
        };
        let mut encoder = replay_frame::frame::FrameEncoder::new();
        let mut decoder = replay_frame::frame::FrameDecoder::new();
        for (hour, available) in [(80, true), (81, false), (82, true)] {
            let mut tables = std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture omitted".into(),
                }
                .into()
            });
            for (id, view) in TABLES.into_iter().zip(if hour == 80 {
                c.tables()
            } else if available {
                [
                    TableView::Complete(TableRows::LandCombatResults(vec![])),
                    TableView::Complete(TableRows::DestroyedDivisions(vec![])),
                    TableView::Complete(TableRows::LandEventCoverage(vec![LandEventCoverageRow {
                        id: 0,
                        epoch: 2,
                        from_hour: 81,
                        to_hour: 82,
                        complete: true,
                        lost_events: 1,
                    }])),
                ]
            } else {
                std::array::from_fn(|_| TableView::Unavailable {
                    reason: "lost observation".into(),
                })
            }) {
                tables[id.index()] = view.into();
            }
            let frame = encoder
                .encode(
                    replay_frame::frame::DecodedFrame {
                        branch: 1,
                        hour,
                        tables,
                        metrics: Default::default(),
                    },
                    false,
                )
                .unwrap();
            let mut bytes = Vec::new();
            frame.encode(&mut bytes).unwrap();
            let mut input = replay_data::codec::Decoder::new(&bytes);
            let wire = replay_frame::frame::Frame::decode(&mut input).unwrap();
            let decoded = decoder.advance(&wire).unwrap();
            let table = &*decoded.tables[TableId::DestroyedDivisions.index()];
            if hour == 80 {
                assert_eq!(
                    table,
                    &TableView::Complete(TableRows::DestroyedDivisions(dead.clone()))
                );
            } else if available {
                assert_eq!(
                    table,
                    &TableView::Complete(TableRows::DestroyedDivisions(vec![]))
                );
            } else {
                assert!(matches!(table, TableView::Unavailable { .. }));
            }
        }
    }
}
