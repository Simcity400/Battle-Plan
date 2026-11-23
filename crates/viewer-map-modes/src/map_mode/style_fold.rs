use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::value_index::ValueIndex;
use replay_data::rows::{Recorded, StateRow};
use replay_frame::frame::ViewIdentity;
use std::sync::Weak;

type Paint = ValueIndex<u32, Style>;
#[derive(Default)]
pub(super) struct Styles {
    bank: Option<Weak<MapData>>,
    states: Option<Categories>,
    manpower: Option<Population>,
    countries: super::country_fold::CountryLayers,
    selected_states: super::state_fold::SelectedStates,
}
pub(super) struct Binding {
    branch: u64,
    campaign: ViewIdentity,
    bank: Weak<MapData>,
}
impl Binding {
    pub(super) fn new(inputs: &Inputs, bank: Weak<MapData>) -> Self {
        Self {
            branch: inputs.frame.branch,
            campaign: inputs.frame.tables[TableId::Campaign.index()].identity(),
            bank,
        }
    }
    pub(super) fn accepts(&self, inputs: &Inputs, bank: Option<&Weak<MapData>>) -> bool {
        self.branch == inputs.frame.branch
            && self
                .campaign
                .is(&inputs.frame.tables[TableId::Campaign.index()])
            && bank.is_some_and(|bank| {
                self.bank.ptr_eq(bank)
                    && bank.strong_count() != 0
                    && bank.as_ptr() == inputs.map as *const MapData
            })
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Category {
    style: Option<Style>,
    refusal: usize,
}
struct Categories {
    binding: Binding,
    rows: RowFold<Recorded<Option<u32>>>,
    tokens: RowFold<Category>,
    by_token_state: ValueIndex<(u32, u32), ()>,
    outcomes: ValueIndex<u32, Category>,
    counts: [usize; 4],
    styles: Paint,
}
struct Population {
    binding: Binding,
    rows: RowFold<Recorded<i32>>,
    missing: usize,
    styles: Paint,
}
impl Styles {
    pub(super) fn country(
        &mut self,
        variant: Variant,
        inputs: &Inputs,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<ModeLayer>> {
        self.countries
            .read(variant, inputs, self.bank.as_ref(), warnings)
    }
    pub(super) fn operatives(
        &mut self,
        inputs: &Inputs,
        selected: u32,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<Paint>> {
        self.selected_states
            .operatives(inputs, selected, self.bank.as_ref(), warnings)
    }
    pub(super) fn occupation(
        &mut self,
        inputs: &Inputs,
        selected: u32,
        compliance: bool,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<Paint>> {
        self.selected_states
            .occupation(inputs, selected, compliance, self.bank.as_ref(), warnings)
    }
    pub(super) fn bind(&mut self, map: &Arc<MapData>) {
        self.bank = Some(Arc::downgrade(map));
    }
    pub(super) fn manpower(
        &mut self,
        inputs: &Inputs,
        rows: &[StateRow],
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<Paint>> {
        let old = self
            .manpower
            .as_ref()
            .filter(|old| old.binding.accepts(inputs, self.bank.as_ref()));
        let rows = RowFold::read(
            inputs.frame,
            TableId::States,
            rows,
            fields(TableId::States, &["population"]),
            old.map(|old| &old.rows),
            |row| Ok(row.population),
        )?;
        let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
        let mut missing = old.map_or(0, |old| old.missing);
        for &key in rows.edits.iter() {
            if old.and_then(|old| old.rows.values.get(&key)) == Some(&Recorded::NotRecorded) {
                missing -= 1;
            }
            match rows.values.get(&key) {
                Some(Recorded::Value(population)) => {
                    styles = styles.inserted(
                        key as u32 + 1,
                        style(manpower_color(&inputs.content.defines, *population)),
                    )?
                }
                Some(Recorded::NotRecorded) => {
                    missing += 1;
                    styles = styles.removed(&(key as u32 + 1))?;
                }
                None => styles = styles.removed(&(key as u32 + 1))?,
            }
        }
        if let Some(bank) = self.bank.clone() {
            self.manpower = Some(Population {
                binding: Binding::new(inputs, bank),
                rows,
                missing,
                styles: styles.clone(),
            });
        }
        if missing != 0 {
            warnings.push("The manpower map mode needs each state's population, which this recording does not hold".to_owned());
            return Ok(None);
        }
        Ok(Some(styles))
    }
    pub(super) fn states(
        &mut self,
        inputs: &Inputs,
        rows: &[StateRow],
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<Paint>> {
        let old = self
            .states
            .as_ref()
            .filter(|old| old.binding.accepts(inputs, self.bank.as_ref()));
        let rows = RowFold::read(
            inputs.frame,
            TableId::States,
            rows,
            fields(TableId::States, &["category"]),
            old.map(|old| &old.rows),
            |row| Ok(row.category),
        )?;
        let lexical = viewer_foundation::tables::rows::<tables::LexicalTokens>(inputs.frame)?;
        let lexical_rows = lexical.rows();
        let view = &inputs.frame.tables[TableId::LexicalTokens.index()];
        let mask = fields(TableId::LexicalTokens, &["literal"]);
        let unchanged = old.is_some_and(|old| old.tokens.identity.fields_unchanged(view, mask));
        if !unchanged {
            let ordered = match old.and_then(|old| view.changes_for(&old.tokens.identity, mask)) {
                Some(facts) => facts.changed().iter().all(|edit| {
                    lexical_rows
                        .binary_search_by_key(&edit.key, replay_data::rows::RowCodec::key)
                        .is_ok_and(|at| {
                            (at == 0 || lexical_rows[at - 1].token < lexical_rows[at].token)
                                && (at + 1 == lexical_rows.len()
                                    || lexical_rows[at].token < lexical_rows[at + 1].token)
                        })
                }),
                None => lexical_rows.is_sorted_by(|a, b| a.token < b.token),
            };
            if !ordered {
                return Err(invalid(format!(
                    "hour {} LexicalTokens rows are not in token order",
                    inputs.frame.hour
                )));
            }
        }
        let tokens = RowFold::read(
            inputs.frame,
            TableId::LexicalTokens,
            lexical_rows,
            mask,
            old.map(|old| &old.tokens),
            |row| {
                let name = (row.token != 0)
                    .then(|| std::str::from_utf8(&row.literal).ok())
                    .flatten()
                    .filter(|name| !name.contains('\0'));
                let Some(name) = name else {
                    return Ok(Category {
                        style: None,
                        refusal: 2,
                    });
                };
                Ok(
                    match inputs
                        .content
                        .category_colors
                        .get(name.to_ascii_lowercase().as_bytes())
                    {
                        Some(Ok(color)) => Category {
                            style: Some(style(color.unwrap_or(STATE_WITHOUT_CATEGORY))),
                            refusal: 4,
                        },
                        _ => Category {
                            style: None,
                            refusal: 3,
                        },
                    },
                )
            },
        )?;
        let mut by_token_state =
            old.map_or_else(ValueIndex::default, |old| old.by_token_state.clone());
        let mut outcomes = old.map_or_else(ValueIndex::default, |old| old.outcomes.clone());
        let mut counts = old.map_or([0; 4], |old| old.counts);
        let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
        let mut dirty = ValueIndex::default();
        for &key in rows.edits.iter() {
            if let Some(Recorded::Value(Some(token))) =
                old.and_then(|old| old.rows.values.get(&key))
            {
                by_token_state = by_token_state.removed(&(*token, key as u32))?;
            }
            if let Some(Recorded::Value(Some(token))) = rows.values.get(&key) {
                by_token_state = by_token_state.inserted((*token, key as u32), ())?;
            }
            dirty = dirty.inserted(key as u32, ())?;
        }
        for &token in tokens.edits.iter() {
            for (&(_, state), _) in by_token_state
                .iter_from(&(token as u32, 0))
                .take_while(|(key, _)| key.0 == token as u32)
            {
                dirty = dirty.inserted(state, ())?;
            }
        }
        for &state in dirty.keys() {
            if let Some(previous) = outcomes.get(&state) {
                if previous.refusal < 4 {
                    counts[previous.refusal] -= 1;
                }
            }
            let outcome = rows.values.get(&(state as u128)).map(|value| match value {
                Recorded::NotRecorded => Category {
                    style: None,
                    refusal: 0,
                },
                Recorded::Value(None) => Category {
                    style: None,
                    refusal: 1,
                },
                Recorded::Value(Some(token)) => tokens
                    .values
                    .get(&(*token as u128))
                    .copied()
                    .unwrap_or(Category {
                        style: None,
                        refusal: 2,
                    }),
            });
            if let Some(outcome) = outcome {
                if outcome.refusal < 4 {
                    counts[outcome.refusal] += 1;
                }
                outcomes = outcomes.inserted(state, outcome)?;
            } else {
                outcomes = outcomes.removed(&state)?;
            }
            styles = match outcome.and_then(|outcome| outcome.style) {
                Some(style) => styles.inserted(state + 1, style)?,
                None => styles.removed(&(state + 1))?,
            };
        }
        if let Some(bank) = self.bank.clone() {
            self.states = Some(Categories {
                binding: Binding::new(inputs, bank),
                rows,
                tokens,
                by_token_state,
                outcomes,
                counts,
                styles: styles.clone(),
            });
        }
        if counts[0] != 0 {
            warnings.push("The states map mode needs each state's live category, which this recording does not hold".to_owned());
            return Ok(None);
        }
        for (count, reason) in counts[1..].iter().zip([
            "whose category read was refused",
            "whose category token the recorded dictionary does not name",
            "whose category the content does not declare or colour",
        ]) {
            if *count != 0 {
                warnings.push(format!(
                    "The states map mode leaves {count} states unfilled {reason}"
                ));
            }
        }
        Ok(Some(styles))
    }
}
