use super::*;
use dashboard_base::products::at_hour::{Edits, Held};
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) fn top_missing(count: usize, first: Option<&LeaderWithheld>, known_skill: bool, unknown_skill: bool) -> Option<String> {
    if let Some(first) = first {
        let noun = if count == 1 { "general" } else { "generals" };
        Some(format!("{count} {noun} could not be shown, so no top general is known: {}", first.reason))
    } else if unknown_skill {
        Some(if known_skill { "Some generals' skills were not recorded at this hour, so no top general is known" }
             else { "Generals' skills were not recorded at this hour, so no top general is known" }.into())
    } else { None }
}

struct Rank {
    handle: u64,
    skill: Option<i32>,
    battles: Battles,
    components: [i32; 4],
    name: Option<String>,
}

impl Rank {
    fn of(leader: &EvaluatedLeader<'_>) -> Self {
        Self { handle: leader.handle, skill: leader.skill, battles: leader.battles, components: leader.row.components, name: leader.name.clone() }
    }

    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        leader_rank_cmp(self.skill, self.battles.won, self.battles.fought, self.components, self.name.as_deref(), self.handle,
            other.skill, other.battles.won, other.battles.fought, other.components, other.name.as_deref(), other.handle)
    }
}

#[derive(Default)]
pub(super) struct Notes {
    leaders: HashMap<u64, catalogue::OwnRefusals>,
    roles: HashMap<u64, catalogue::OwnRefusals>,
    traits: HashMap<String, catalogue::OwnRefusals>,
    icons: catalogue::OwnRefusals,
}

impl Notes {
    pub(super) fn leader(&mut self, key: u64, read: impl FnOnce()) {
        Self::observe(&mut self.leaders, key, read);
    }

    pub(super) fn role(&mut self, key: u64, read: impl FnOnce() -> LeaderRole) -> LeaderRole {
        Self::observe(&mut self.roles, key, read)
    }

    pub(super) fn trait_info(&mut self, key: &str, read: impl FnOnce() -> LeaderTrait) -> LeaderTrait {
        Self::observe(&mut self.traits, key.to_owned(), read)
    }

    fn observe<K: Eq + std::hash::Hash, T>(map: &mut HashMap<K, catalogue::OwnRefusals>, key: K, read: impl FnOnce() -> T) -> T {
        let (value, noted) = catalogue::capture(read);
        if !noted.diagnostics().is_empty() || !noted.fallbacks().is_empty() { map.insert(key, noted); }
        else { map.remove(&key); }
        value
    }

    pub(super) fn icons<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let (value, notes) = catalogue::capture(f);
        self.icons = notes;
        value
    }

    fn combined(&self) -> catalogue::OwnRefusals {
        let mut notes = self.icons.clone();
        for item in self.leaders.values().chain(self.roles.values()).chain(self.traits.values()) { notes.merge(item); }
        notes
    }

    fn forget(&mut self, keys: &BTreeSet<u64>) {
        for key in keys { self.leaders.remove(key); self.roles.remove(key); }
    }
}

type Top = Result<u64, Option<String>>;

#[derive(Default)]
pub(super) struct Ranking {
    order: Vec<u64>,
    shown: HashMap<u64, Rank>,
    withheld: BTreeMap<u64, LeaderWithheld>,
    people: HashMap<u32, BTreeSet<u64>>,
    tokens: HashMap<u32, BTreeSet<u32>>,
    character: HashMap<u64, u32>,
    spellings: HashMap<u32, Vec<u32>>,
    traits: Vec<LeaderTrait>,
    pending: Option<(i32, Top)>,
    revision: u64,
    pub(super) notes: Notes,
}

impl Ranking {
    pub(super) fn seed(frame: &DecodedFrame, evaluated: &EvaluatedLeaders<'_>, context: &LeadersContext, notes: Notes) -> Self {
        let mut rank = Self {
            revision: UnitLeaders::context_revision(context),
            notes,
            order: evaluated.leaders.iter().map(|l| l.handle).collect(),
            shown: evaluated.leaders.iter().map(|l| (l.handle, Rank::of(l))).collect(),
            withheld: evaluated.withheld.iter().map(|l| (l.reference.parse().expect("recorded handle"), l.clone())).collect(),
            ..Self::default()
        };
        if let Ok(rows) = frame.complete::<tables::UnitLeaders>() {
            for row in rows {
                rank.people.entry(row.character_id).or_default().insert(handle(row));
                rank.character.insert(handle(row), row.character_id);
            }
        }
        if let Ok(rows) = frame.complete::<tables::Characters>() {
            for row in rows { rank.watch_spelling(row); }
        }
        rank
    }

    fn watch_spelling(&mut self, row: &CharacterRow) {
        let mut tokens = Vec::new();
        if let Ok(id) = u32::try_from(row.name_token_id) { tokens.push(id); }
        if row.template_present != 0 { tokens.push(row.template_tag); }
        for &token in &tokens { self.tokens.entry(token).or_default().insert(row.id); }
        self.spellings.insert(row.id, tokens);
    }

    pub(super) fn seed_traits(&mut self, reply: &UnitLeadersReply) {
        self.traits.clone_from(&reply.traits);
    }

    fn affected(&mut self, frame: &DecodedFrame, edits: &Edits, winner: Option<u64>, context: &LeadersContext) -> Option<BTreeSet<u64>> {
        if !edits.replaced.is_empty() || !frame.tables[TableId::Clock.index()].held().is_not_selected() { return None; }
        let leaders = frame.complete::<tables::UnitLeaders>().ok()?;
        let mut affected = BTreeSet::new();
        let visibility = context.visibility.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.revision != visibility.revision {
            if self.revision.checked_add(1) != Some(visibility.revision) { return None; }
            affected.extend(&visibility.changed);
            self.revision = visibility.revision;
        }
        drop(visibility);
        for (&table, rows) in &edits.tables {
            match table {
                TableId::UnitLeaders => {
                    for &key in rows.keys() {
                        let handle_key = key as u64;
                        affected.insert(handle_key);
                        let current = leaders.binary_search_by_key(&handle_key, handle).ok().map(|i| leaders[i].character_id);
                        if current == self.character.get(&handle_key).copied() { continue; }
                        if let Some(old) = self.character.remove(&handle_key) && let Some(handles) = self.people.get_mut(&old) {
                            handles.remove(&handle_key);
                            if handles.is_empty() { self.people.remove(&old); }
                        }
                        if let Some(id) = current {
                            self.character.insert(handle_key, id);
                            self.people.entry(id).or_default().insert(handle_key);
                        }
                    }
                }
                TableId::LeaderBattleTotals => {
                    for key in rows.keys() {
                        let id = (key >> 32) as u32;
                        let first = leaders.partition_point(|r| r.id < id);
                        affected.extend(leaders[first..].iter().take_while(|r| r.id == id).map(handle));
                    }
                }
                TableId::UnitLeaderTraits | TableId::LeaderTraitDates => affected.extend(rows.keys().map(|key| (key >> 32) as u64)),
                TableId::UnitLeaderTraitExperience => {
                    let experience = dashboard_base::products::at_hour::field_mask(table, &["experience"]);
                    affected.extend(rows.iter().filter(|(k, fields)| **k & (1 << 32) != 0 || **fields & !experience != 0).map(|(k, _)| (k >> 33) as u64));
                }
                TableId::Characters => {
                    let characters = frame.complete::<tables::Characters>().ok()?;
                    for &key in rows.keys() {
                        let id = key as u32;
                        if let Some(handles) = self.people.get(&id) { affected.extend(handles); }
                        for token in self.spellings.remove(&id).unwrap_or_default() {
                            if let Some(people) = self.tokens.get_mut(&token) {
                                people.remove(&id);
                                if people.is_empty() { self.tokens.remove(&token); }
                            }
                        }
                        if let Ok(index) = characters.binary_search_by_key(&id, |row| row.id) { self.watch_spelling(&characters[index]); }
                    }
                }
                TableId::LexicalTokens => {
                    for &key in rows.keys() {
                        if let Some(people) = self.tokens.get(&(key as u32)) {
                            for id in people { if let Some(handles) = self.people.get(id) { affected.extend(handles); } }
                        }
                    }
                    if let Some(winner) = winner && career::progress_token_changed(frame, winner, rows) { affected.insert(winner); }
                }
                _ => return None,
            }
        }
        Some(affected)
    }

    fn battle_changes(&mut self, frame: &DecodedFrame, at: &At<'_, LeadersContext>, keys: &BTreeSet<u64>) -> io::Result<Vec<(u64, usize, usize, Battles)>> {
        let query = at.context.query(frame.branch)?;
        let mut ids: Vec<_> = keys.iter().filter(|key| self.shown.contains_key(key)).map(|key| (key >> 32) as u32).collect();
        ids.dedup();
        if ids.is_empty() { return Ok(Vec::new()); }
        let tally = at.context.tally(&query, frame, Asked::Ids(&ids))?;
        let mut moves = Vec::new();
        for &key in keys {
            let Some(old) = self.shown.get(&key) else { continue };
            let id = (key >> 32) as u32;
            let counts = tally.battles(id);
            if counts == old.battles { continue; }
            let from = self.order.binary_search_by(|h| self.shown[h].cmp(old)).expect("ranked leader");
            self.order.remove(from);
            self.shown.get_mut(&key).expect("ranked leader").battles = counts;
            let rank = &self.shown[&key];
            let to = self.order.partition_point(|h| self.shown[h].cmp(rank).is_lt());
            self.order.insert(to, key);
            moves.push((key, from, to, counts));
        }
        Ok(moves)
    }

    fn change(&mut self, keys: &BTreeSet<u64>, evaluated: &EvaluatedLeaders<'_>) -> (Vec<usize>, Vec<usize>) {
        let mut removed = Vec::new();
        for key in keys {
            self.withheld.remove(key);
            if let Some(old) = self.shown.get(key) {
                let i = self.order.binary_search_by(|h| self.shown[h].cmp(old)).expect("ranked leader");
                removed.push(i);
            }
        }
        removed.sort_unstable_by(|a, b| b.cmp(a));
        for &i in &removed { self.shown.remove(&self.order.remove(i)); }
        for row in &evaluated.withheld { self.withheld.insert(row.reference.parse().expect("recorded handle"), row.clone()); }
        let mut inserted = Vec::new();
        for leader in &evaluated.leaders {
            let rank = Rank::of(leader);
            let i = self.order.partition_point(|h| self.shown[h].cmp(&rank).is_lt());
            self.order.insert(i, leader.handle);
            self.shown.insert(leader.handle, rank);
            inserted.push(i);
        }
        (removed, inserted)
    }

    fn top(&self) -> Top {
        let first = self.order.first().map(|h| &self.shown[h]);
        let unknown = self.order.last().is_some_and(|h| self.shown[h].skill.is_none());
        if let Some(reason) = top_missing(self.withheld.len(), self.withheld.values().next(), first.is_some_and(|r| r.skill.is_some()), unknown) {
            return Err(Some(reason));
        }
        first.map(|r| r.handle).ok_or(None)
    }

    pub(super) fn take_top(&mut self, hour: i32) -> Option<Top> {
        let (_, top) = self.pending.take().filter(|(at, _)| *at == hour)?;
        catalogue::credit(&self.notes.combined());
        Some(top)
    }

    pub(super) fn top_step(&mut self, frame: &DecodedFrame, params: &UnitLeadersParams, at: &At<'_, LeadersContext>, edits: &Edits, data: &Held<'_>) -> io::Result<bool> {
        let before = self.top();
        let same_proof = self.revision == UnitLeaders::context_revision(at.context);
        let Some(keys) = self.affected(frame, edits, before.as_ref().ok().copied(), at.context) else { return Ok(false) };
        if keys.is_empty() { return Ok(true); }
        if same_proof && edits.tables.len() == 1 && edits.tables.contains_key(&TableId::LeaderBattleTotals) {
            let changes = self.battle_changes(frame, at, &keys)?;
            let next = self.top();
            if before == next && !changes.iter().any(|(key, ..)| next.as_ref().ok() == Some(key)) { return Ok(true); }
            self.pending = Some((at.hour, next));
            return Ok(false);
        }
        let prior_notes = self.notes.combined();
        self.notes.forget(&keys);
        let evaluated = UnitLeaders::evaluate_keys(frame, params, at, Some(&keys), Some(&mut self.notes))?;
        if evaluated.missing.is_some() { return Ok(false); }
        self.change(&keys, &evaluated);
        let next = self.top();
        if prior_notes == self.notes.combined() {
            if before == next && next.as_ref().is_ok_and(|h| !keys.contains(h)) { return Ok(true); }
            if let Err(missing) = &next && data.get("missing").and_then(Value::as_str) == missing.as_deref() && data["reference"].is_null() { return Ok(true); }
        }
        self.pending = Some((at.hour, next));
        Ok(false)
    }

    pub(super) fn board_step(&mut self, frame: &DecodedFrame, params: &UnitLeadersParams, at: &At<'_, LeadersContext>, edits: &Edits, data: &mut Held<'_>) -> io::Result<bool> {
        let same_proof = self.revision == UnitLeaders::context_revision(at.context);
        let Some(keys) = self.affected(frame, edits, None, at.context) else { return Ok(false) };
        if keys.is_empty() { return Ok(true); }
        if same_proof && edits.tables.len() == 1 && edits.tables.contains_key(&TableId::LeaderBattleTotals) {
            let moves = self.battle_changes(frame, at, &keys)?;
            if moves.is_empty() { return Ok(true); }
            let rows = data["leaders"].as_array_mut().expect("leader board");
            let mut reordered = false;
            for (_, from, to, counts) in moves {
                if from != to {
                    let row = rows.remove(from);
                    rows.insert(to, row);
                    reordered = true;
                }
                let row = &mut rows[to];
                row["fought"] = counts.fought.into();
                row["won"] = counts.won.into();
                row["lost"] = counts.lost.into();
            }
            if reordered { self.order_traits(data)?; }
            return Ok(true);
        }
        let prior_notes = self.notes.combined();
        self.notes.forget(&keys);
        let evaluated = UnitLeaders::evaluate_keys(frame, params, at, Some(&keys), Some(&mut self.notes))?;
        if evaluated.missing.is_some() { return Ok(false); }
        let withheld_changed = keys.iter().any(|k| self.withheld.contains_key(k)) || !evaluated.withheld.is_empty();
        let (removed, inserted) = self.change(&keys, &evaluated);
        if removed.is_empty() && inserted.is_empty() && !withheld_changed && self.notes.combined() == prior_notes { return Ok(true); }
        let update = UnitLeaders::board(frame, evaluated, at, Some(&mut self.notes))?;
        let mut catalog = HashMap::new();
        for (i, held) in self.traits.iter().enumerate() { catalog.insert(held.token.clone(), i as u32); }
        let mut lookup = Vec::new();
        for item in update.traits {
            let i = *catalog.entry(item.token.clone()).or_insert_with(|| { self.traits.push(item); (self.traits.len() - 1) as u32 });
            lookup.push(i);
        }
        let rows = data["leaders"].as_array_mut().expect("leader board");
        for i in removed { rows.remove(i); }
        for (i, mut row) in inserted.into_iter().zip(update.leaders) {
            if let Some(traits) = &mut row.traits { for t in traits { *t = lookup[*t as usize]; } }
            rows.insert(i, serde_json::to_value(row)?);
        }
        self.order_traits(data)?;
        self.notes.traits.retain(|token, _| self.traits.iter().any(|t| t.token == *token));
        let next_notes = self.notes.combined();
        if next_notes != prior_notes { data.warnings(update.warnings, &next_notes); }
        if withheld_changed { data["withheld"] = serde_json::to_value(self.withheld.values().collect::<Vec<_>>())?; }
        Ok(true)
    }

    fn order_traits(&mut self, data: &mut Held<'_>) -> io::Result<()> {
        let rows = data["leaders"].as_array().expect("leader board");
        let mut order = Vec::new();
        let mut indices = vec![None; self.traits.len()];
        for row in rows.iter() {
            for t in row["traits"].as_array().into_iter().flatten() {
                let old = t.as_u64().expect("trait index") as usize;
                indices[old].get_or_insert_with(|| { order.push(old); order.len() - 1 });
            }
        }
        if order.len() != self.traits.len() || order.iter().copied().ne(0..order.len()) || data["traits"].as_array().is_none_or(|a| a.len() != self.traits.len()) {
            for row in data["leaders"].as_array_mut().expect("leader board") {
                if let Some(traits) = row["traits"].as_array_mut() { for t in traits { *t = indices[t.as_u64().expect("trait index") as usize].expect("used trait").into(); } }
            }
            self.traits = order.into_iter().map(|i| self.traits[i].clone()).collect();
            data["traits"] = serde_json::to_value(&self.traits)?;
        }
        Ok(())
    }
}
