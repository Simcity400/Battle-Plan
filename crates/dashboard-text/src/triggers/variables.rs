//! Script variables over the recorded hour: parse operand text
//! ([`installed_content_core::scoped_variable`]), evaluate its value,
//! select its store and build its key.
//!
//! The persistent stores (global, country, state and character) are the
//! recorded `ScriptVariables` rows; a project's and an industrial
//! organisation's are their own rows' recorded stores. The thread-local store holds known local
//! setter results and the caller's injected temps, including raid event
//! scope temps, in a trigger-check context. Starting a new context
//! clears the store when no context is active. A read the thread-local store
//! may answer otherwise (an unsupported writer's results,
//! or any temp outside a context) is refused by
//! name, as are unreproduced built-in game variables ([`installed_content_core::variable_builtins`],
//! looked up before the stored keys), and other runtime-only
//! forms. Saved targets select stores through the caller's event graph when
//! present, then the recorded global registry; flat callers retain explicit
//! refusals for event-chain links they cannot describe.
use crate::triggers::Failure;

// Complete country rosters bound these lookups, so present partial rows remain exact.
fn localisation_status(
    frame: replay_frame::frame::HourFrame<'_>,
    table: replay_data::rows::TableId,
    status: replay_frame::frame::OperandStatus<'_>,
) -> Result<(), Failure> {
    use replay_frame::frame::OperandStatus;
    match status {
        OperandStatus::Complete | OperandStatus::Truncated(_) => Ok(()),
        OperandStatus::WrongLayout | OperandStatus::TruncatedWrongLayout(_) => {
            Err(format!("{} holds rows of another kind", table.words()).into())
        }
        _ => status
            .complete(frame.hour(), table)
            .map_err(|why| super::refusal::input_error(table, why)),
    }
}

use installed_content_core::{
    parser::Item, parser::Operator, scope::Keyword as ScopeKeyword, scope::Names, scoped_variable,
    scoped_variable::Constant, scoped_variable::Link, scoped_variable::ScopedVariable,
    variable_builtins,
};

use super::{
    Scope, Scopes,
    eval::Evaluator,
    saved_event_targets::Reader,
    world::{NULL_STATE, fixed_round},
};
use crate::state_resources::ONE;

mod random;
pub(super) mod temps;

/// Coded triggers whose evaluation writes the thread-local store:
/// scalar setters and arithmetic, rounding and clamping, array writers,
/// iterators, element selection and career-profile setters.
pub(super) const TEMP_WRITERS: [&str; 19] = [
    "add_to_temp_array",
    "add_to_temp_variable",
    "all_of",
    "any_of",
    "career_profile_set_temp_playthrough_variable",
    "career_profile_set_temp_variable",
    "clamp_temp_variable",
    "clear_temp_array",
    "divide_temp_variable",
    "find_highest_in_array",
    "find_lowest_in_array",
    "modulo_temp_variable",
    "multiply_temp_variable",
    "remove_from_temp_array",
    "resize_temp_array",
    "round_temp_variable",
    "set_temp_variable",
    "set_temp_variable_to_random",
    "subtract_from_temp_variable",
];

/// What the thread-local store may hold besides the caller's temps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Temps {
    /// An active trigger-check context: only the caller's temps and what
    /// the evaluation's own setters write.
    #[default]
    Fresh,
    /// Outside any context (a localisation's `[?...]`,
    /// which begins none): closing a variable context clears only
    /// the active flag, so the thread keeps the temps its last context set.
    Stale,
    /// A refresh invoked inside an unrecorded outer script context. Even
    /// inside a nested check, an unwritten name may already have a TLS value.
    Inherited,
}

pub(super) fn inherited_temps_failure() -> Failure {
    "force_update_map_mode caller temporary variables are not recorded".into()
}

/// A variable store, as `ScriptVariables`
/// keys it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Store {
    /// Global store (scope kind 0).
    Global,
    /// Country store of a canonical slot (kind 1, by the raw country id).
    Country(u32),
    /// State store (kind 2).
    State(u32),
    /// Character store (kind 3, by the character id).
    Character(u32),
    Project {
        owner: u32,
        id: u32,
    },
    /// An industrial organisation's persistent store, by its full reference
    /// (`IndustrialOrganisationDetails.variables`).
    IndustrialOrganisation(u64),
}

impl Store {
    /// The store's built-in table.
    fn builtins(self) -> &'static [&'static str] {
        match self {
            Self::Global => &variable_builtins::GLOBAL,
            Self::Country(_) => &variable_builtins::COUNTRY,
            Self::State(_) => &variable_builtins::STATE,
            Self::Character(_) => &variable_builtins::CHARACTER,
            Self::IndustrialOrganisation(_) => &variable_builtins::ORGANISATION,
            Self::Project { .. } => &variable_builtins::PROJECT,
        }
    }
}

/// An evaluated operand: a raw fixed5 value, or a scope object whose
/// encoded value is computed only where it is needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Raw(i64),
    Object(Scope),
}

/// Where an operand's store is, and the scope object encoded
/// for an operand without a name.
struct Resolved {
    store: Option<Store>,
    object: Option<Scope>,
}

/// The content facts the grammar reads, over the recorded hour.
struct Content<'c, 'e, 'a> {
    evaluator: &'c Evaluator<'e, 'a>,
}

impl scoped_variable::Content for Content<'_, '_, '_> {
    fn is_country(&self, tag: &str) -> bool {
        Names::is_country(self.evaluator.world().recorded(), tag)
    }

    fn constant(&self, parts: &[&str]) -> Constant {
        match self.evaluator.world().script_constants() {
            Ok(constants) => constants.value(parts),
            Err(why) => Constant::Unset(format!(
                "constant:{}: the script constants are unread: {why}",
                parts.join(".")
            )),
        }
    }
}

impl<'e, 'a> Evaluator<'e, 'a> {
    /// Parse an operand text.
    pub fn operand(&self, text: &[u8]) -> Result<ScopedVariable, Failure> {
        let text = std::str::from_utf8(text).map_err(|_| {
            format!(
                "the operand {} is not UTF-8, so its grammar cannot be read",
                String::from_utf8_lossy(text)
            )
        })?;
        scoped_variable::parse(text, &Content { evaluator: self }).ok_or_else(|| {
            format!(
                "an operand nests defaults, indices, targets and prefixes more than {} levels deep, which is not evaluated",
                scoped_variable::MAX_NESTING
            )
            .into()
        })
    }

    /// Evaluate an operand text in `scopes`,
    /// as a raw fixed5 value.
    pub fn scoped_value(&self, text: &[u8], scopes: Scopes) -> Result<i64, Failure> {
        let within =
            |why: Failure| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text)));
        let operand = self.operand(text).map_err(within)?;
        self.evaluate_operand(&operand, scopes, None)
            .and_then(|value| self.raw(value))
            .map_err(within)
    }

    pub(crate) const NULL_IDEOLOGY: &'static str =
        "ideology database null-object identity is unrecorded";

    /// Admit a named group or scoped variable;
    /// select its ideology ID, then a decoded country's ruling group,
    /// evaluating only once.
    pub(crate) fn ideology_specifier(
        &self,
        operand: Option<&[u8]>,
        scopes: Scopes,
        is_group: impl Fn(&[u8]) -> Result<bool, Failure>,
    ) -> Result<(u32, &'a [u8]), Failure> {
        let operand = operand.ok_or(Self::NULL_IDEOLOGY)?;
        let world = self.world();
        if is_group(operand)? {
            return world.named_ideology(operand);
        }
        let text = std::str::from_utf8(operand).map_err(|_| "ideology operand is not UTF-8")?;
        if !(operand.starts_with(b"var:")
            || [b"root".as_slice(), b"prev", b"this", b"from"]
                .iter()
                .any(|scope| operand.eq_ignore_ascii_case(scope))
            || operand.len() == 3 && Names::is_country(world.recorded(), text))
        {
            return Err(Self::NULL_IDEOLOGY.into());
        }
        let value = self.scoped_value(operand, scopes)?;
        let id = (value / ONE) as u32;
        if let Some(token) = world.ideology_token(id)? {
            if !is_group(token)? {
                return Err(format!(
                    "ideology {} is absent from recorded content",
                    String::from_utf8_lossy(token)
                )
                .into());
            }
            return Ok((id, token));
        }
        // Decode the already evaluated value; a repeated authored read could
        // consume random/alias/temp state a second time.
        let constant = installed_content_core::fixed5::fixed5_to_string(value, 5);
        let Some(country) = self.scoped_country(constant.as_bytes(), scopes)? else {
            return Err(Self::NULL_IDEOLOGY.into());
        };
        let (id, token) = world.scoped_ruling_ideology(country)?;
        if !is_group(token)? {
            return Err(format!(
                "ideology {} is absent from recorded content",
                String::from_utf8_lossy(token)
            )
            .into());
        }
        Ok((id, token))
    }

    /// Numeric localisation uses the character fallback flag.
    /// Nested defaults, targets and indices still use the ordinary evaluator.
    pub fn scoped_localisation_value(&self, text: &[u8], scopes: Scopes) -> Result<i64, Failure> {
        let within =
            |why: Failure| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text)));
        let original =
            std::str::from_utf8(text).map_err(|_| within("operand is not UTF-8".into()))?;
        let fallback = original
            .strip_prefix("var:")
            .or_else(|| original.strip_prefix("temp_var:"))
            .unwrap_or(original);
        let operand = self.operand(text).map_err(within)?;
        self.evaluate_operand_with_fallback(&operand, scopes, None, Some(fallback))
            .and_then(|value| self.raw(value))
            .map_err(within)
    }

    /// Search the parsing scope's positive country, including
    /// retired records.
    pub(crate) fn admit_localisation_parse_character(
        &self,
        candidate: &str,
        scopes: Scopes,
    ) -> Result<(), Failure> {
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => {
                let raw = self
                    .world()
                    .raw_country(country)
                    .ok_or_else(|| -> Failure {
                        format!("{candidate}: the parse scope's country reference is not recorded")
                            .into()
                    })?;
                if (raw as i32) <= 0 {
                    return Ok(());
                }
                country
            }
            Scope::Province(_) => {
                if self
                    .localisation_character_candidates(candidate)?
                    .is_empty()
                {
                    return Ok(());
                }
                return Err(format!(
                    "{candidate}: character-token parsing needs the province scope's unrecorded country carrier"
                ).into());
            }
            _ => return Ok(()),
        };
        const RECORD: u64 =
            replay_data::rows::mask::<replay_data::rows::CountryCharacterRow>(&["character"]);
        const NAME: u64 =
            replay_data::rows::mask::<replay_data::rows::CharacterRow>(&["name_token_id"]);
        let active = super::world::table::<replay_data::rows::tables::CountryCharacters>(
            self.world().tables(),
            RECORD,
        )?;
        let retired = super::world::table::<replay_data::rows::tables::CountryRetiredCharacters>(
            self.world().tables(),
            RECORD,
        )?;
        for record in active.iter().chain(retired.iter()) {
            let record = record.map_err(|why| {
                super::refusal::input_error(replay_data::rows::TableId::CountryCharacters, why)
            })?;
            let Some(id) = record.character else {
                continue;
            };
            let owner = self
                .world()
                .slot(record.country)
                .ok_or_else(|| -> Failure {
                    self.world()
                        .unlinked("country characters/country retired characters", record.country)
                })?;
            if owner != country {
                continue;
            }
            // The roster bounds this lookup. Unrelated truncated manager or
            // dictionary rows cannot shadow this country's parsed operand.
            let frame = self.world().tables();
            localisation_status(
                frame,
                replay_data::rows::TableId::Characters,
                frame.status(replay_data::rows::TableId::Characters),
            )?;
            // Both are keyed: Characters by id, LexicalTokens by token.
            let character = frame
                .held::<replay_data::rows::tables::Characters>(NAME)
                .get(u128::from(id))
                .map_err(|why| {
                    super::refusal::input_error(replay_data::rows::TableId::Characters, why)
                })?
                .ok_or_else(|| -> Failure {
                    format!("{candidate}: rostered character {id} is unread in Characters, so its token could shadow the numeric operand").into()
                })?;
            let Ok(token) = u32::try_from(character.name_token_id) else {
                continue;
            };
            localisation_status(
                frame,
                replay_data::rows::TableId::LexicalTokens,
                frame.status(replay_data::rows::TableId::LexicalTokens),
            )?;
            let tokens = frame.held::<replay_data::rows::tables::LexicalTokens>(0);
            let spelling = super::world::literal(&tokens, token)?.ok_or_else(|| -> Failure {
                format!("{candidate}: rostered character {id}'s name token {token} is unread in lexical tokens").into()
            })?;
            if spelling.eq_ignore_ascii_case(candidate.as_bytes()) {
                return Err(format!(
                    "{candidate}: live character {id} changes numeric localisation's parsed operand; that grammar path is not reproduced"
                ).into());
            }
        }
        Ok(())
    }

    fn localisation_character_candidates(&self, name: &str) -> Result<Vec<u32>, Failure> {
        const NAME: u64 =
            replay_data::rows::mask::<replay_data::rows::CharacterRow>(&["name_token_id"]);
        let characters = super::world::characters(self.world().tables(), NAME)?;
        let mut named = Vec::new();
        for character in characters.iter() {
            let character = character.map_err(|why| {
                super::refusal::input_error(replay_data::rows::TableId::Characters, why)
            })?;
            named.push((character.id, character.name_token_id));
        }
        if named.iter().all(|&(_, token)| token < 0) {
            return Ok(Vec::new());
        }
        let tokens = super::world::lexical(self.world().tables())?;
        let mut matches = Vec::new();
        for (id, token) in named {
            let Ok(token) = u32::try_from(token) else {
                continue;
            };
            let literal = super::world::literal(&tokens, token)?.ok_or_else(|| -> Failure {
                format!("{name}: character {id}'s name token {token} is not spelled in lexical tokens, so native character lookup cannot be decided").into()
            })?;
            if literal.eq_ignore_ascii_case(name.as_bytes()) {
                matches.push(id);
            }
        }
        Ok(matches)
    }

    fn admit_localisation_fallback_character(&self, original: &str) -> Result<(), Failure> {
        let matches = self.localisation_character_candidates(original)?;
        if !matches.is_empty() {
            return Err(format!(
                "{original}: numeric localisation can resolve character tokens {:?} before stored variables; its country/PREV selection is not recorded",
                matches
            ).into());
        }
        Ok(())
    }

    /// Test operand existence, used by effect-list weights.
    pub fn scoped_exists(&self, text: &[u8], scopes: Scopes) -> Result<bool, Failure> {
        self.exists(&self.operand(text)?, scopes)
    }

    /// The scope an operand text names: evaluate, then decode the value,
    /// as a `var:` scope key and a country or state operand read it.
    /// `None` names
    /// nothing.
    pub fn scoped_scope(&self, text: &[u8], scopes: Scopes) -> Result<Option<Scope>, Failure> {
        let within =
            |why: Failure| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text)));
        let operand = self.operand(text).map_err(within)?;
        let value = self
            .evaluate_operand(&operand, scopes, None)
            .map_err(within)?;
        self.decoded(value, scopes).map_err(within)
    }

    /// The decoded scope's country, as a
    /// canonical slot; `None` is country 0.
    pub fn scoped_country(&self, text: &[u8], scopes: Scopes) -> Result<Option<u32>, Failure> {
        Ok(match self.scoped_scope(text, scopes)? {
            Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => Some(slot),
            Some(Scope::Combat) => Some(self.combat_country()?),
            _ => None,
        })
    }

    /// The decoded state, else the value's
    /// `fixed_round` as the game state's state list keeps it
    /// ([`super::World::state_of_value`]); [`NULL_STATE`] for none.
    pub fn scoped_state(&self, text: &[u8], scopes: Scopes) -> Result<u32, Failure> {
        let within =
            |why: Failure| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text)));
        let operand = self.operand(text).map_err(within)?;
        match self
            .evaluate_operand(&operand, scopes, None)
            .map_err(within)?
        {
            Value::Object(Scope::State(id)) => Ok(id),
            value => {
                let raw = self.raw(value).map_err(within)?;
                self.world().state_of_value(raw).map_err(within)
            }
        }
    }

    /// State operand evaluation returns a signed ID without roster validation.
    pub fn scoped_state_id(&self, text: &[u8], scopes: Scopes) -> Result<i32, Failure> {
        let operand = self.operand(text)?;
        let value = self.evaluate_operand(&operand, scopes, None)?;
        if let Some(Scope::State(id)) = self.decoded(value, scopes)?
            && (id as i32) > 0
        {
            return Ok(id as i32);
        }
        Ok(fixed_round(self.raw(value)?) as i32)
    }

    /// Evaluate with `index` overriding the
    /// operand's own `^index`, `char_fallback` clear.
    pub(super) fn evaluate_operand(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
    ) -> Result<Value, Failure> {
        self.evaluate_operand_with_fallback(operand, scopes, index, None)
    }

    fn evaluate_operand_with_fallback(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
        fallback: Option<&str>,
    ) -> Result<Value, Failure> {
        self.operand_value(operand, scopes, index, fallback)
    }

    fn operand_value(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
        fallback: Option<&str>,
    ) -> Result<Value, Failure> {
        // Evaluation checks the literal flag before the other operand fields;
        // constructor literals need not have the parsed flag set.
        if let Some(value) = operand.literal {
            return Ok(Value::Raw(value));
        }
        if let Some(why) = unported(operand) {
            return Err(why.into());
        }
        if let Some(token) = &operand.token {
            return self.token_value(token);
        }
        // Evaluation tests MIO, then project,
        // before its store path; a missing match is the raw value zero.
        for (prefix, name) in [("mio", &operand.mio), ("sp", &operand.special_project)] {
            if let Some(name) = name {
                let country = match scopes.this {
                    Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => slot,
                    Scope::Combat => self.combat_country()?,
                    _ => {
                        return Err(format!(
                            "{prefix}:{name}: scoped-variable evaluation needs THIS country"
                        )
                        .into());
                    }
                };
                if self
                    .world()
                    .raw_country(country)
                    .is_none_or(|raw| raw as i32 <= 0)
                {
                    return Err(format!(
                        "{prefix}:{name}: THIS country is not recorded or is null"
                    )
                    .into());
                }
                return Ok(self
                    .named_carrier(prefix, name, country)?
                    .map_or(Value::Raw(0), Value::Object));
            }
        }
        // No name and no draw: use the resolved scope's encoded value.
        if operand.name.is_empty() && !operand.random {
            let object = self.resolve(operand, scopes, true)?.object;
            if fallback.is_some()
                && matches!(
                    object,
                    Some(Scope::Character(_) | Scope::CountryLeader { .. })
                )
            {
                return Err("numeric localisation's character-first identity encoding needs the character's current unit-leader role, which is not reproduced".into());
            }
            return Ok(match object {
                Some(scope) => Value::Object(scope),
                None => Value::Raw(0),
            });
        }
        if !operand.temp && (!operand.chain.is_empty() || operand.random) {
            return self.stored(operand, scopes, index, fallback);
        }
        let key = self.key(operand, scopes, index)?;
        match self.temp(&key, scopes)? {
            Some(value) => Ok(value),
            // A temp operand reads the thread-local store only; its
            // `?default` is not read.
            None if operand.temp => Ok(Value::Raw(0)),
            None => self.stored(operand, scopes, index, fallback),
        }
    }

    /// Script target indexed array reads, using evaluation's own override path.
    pub fn scoped_array_scopes(
        &self,
        text: &[u8],
        scopes: Scopes,
    ) -> Result<Vec<Option<Scope>>, Failure> {
        let mut values = Vec::new();
        self.visit_scoped_array_scopes(text, scopes, |value| {
            values.push(value);
            Ok(())
        })?;
        Ok(values)
    }

    /// Check each target before evaluating the next array index;
    /// its trigger can write the shared TLS store used by that next index.
    pub(crate) fn visit_scoped_array_scopes(
        &self,
        text: &[u8],
        scopes: Scopes,
        mut visit: impl FnMut(Option<Scope>) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let operand = self.operand(text)?;
        self.visit_array_values(&operand, scopes, |value| {
            visit(self.decoded(value, scopes)?)
        })
    }

    /// Operand existence, `has_variable`: a literal is
    /// present; a temp operand is looked up in the thread-local store only,
    /// any other in its store, then the thread-local store. The built-in
    /// table is not consulted; only stored keys are probed.
    pub(super) fn exists(&self, operand: &ScopedVariable, scopes: Scopes) -> Result<bool, Failure> {
        if let Some(why) = unported(operand) {
            return Err(why.into());
        }
        if operand.literal.is_some() || operand.token.is_some() {
            return Ok(true);
        }
        if operand.random || operand.name.is_empty() {
            return Err(
                "an operand without a name is looked up by an empty key, which is not reproduced"
                    .into(),
            );
        }
        let key = self.key(operand, scopes, None)?;
        if operand.temp {
            return Ok(self.temp(&key, scopes)?.is_some());
        }
        let stored = match self.resolve(operand, scopes, false)?.store {
            Some(store) => self.world().variable(store, &key)?.is_some(),
            None => false,
        };
        Ok(stored || self.temp(&key, scopes)?.is_some())
    }

    /// The array operand's values: `<name>^num` rounded
    /// entries, each read with its index as the
    /// override.
    pub(super) fn array(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
    ) -> Result<Vec<Value>, Failure> {
        let mut values = Vec::new();
        self.visit_array_values(operand, scopes, |value| {
            values.push(value);
            Ok(())
        })?;
        Ok(values)
    }

    fn visit_array_values(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        mut visit: impl FnMut(Value) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let num = ScopedVariable {
            parsed: true,
            num: true,
            ..ScopedVariable::default()
        };
        let count = self.evaluate_operand(operand, scopes, Some(&num))?;
        let count = fixed_round(self.raw(count)?);
        self.visit_indexed_values(operand, scopes, count, |value| {
            visit(value)?;
            Ok(false)
        })?;
        Ok(())
    }

    /// Array membership: signed truncated i32
    /// length, exact fixed5 equality, and no reads after the first match.
    pub(super) fn is_in_array(&self, block: &[Item], scopes: Scopes) -> Result<bool, Failure> {
        let (mut array, mut count, mut wanted) = (temps::zero(), temps::zero(), temps::zero());
        for item in block {
            // Reading assigns size/index only to the length operand,
            // which array membership does not evaluate.
            if item.keyword_is("size") || item.keyword_is("index") {
                continue;
            }
            if item.keyword_is("value") {
                temps::assign(self, &mut wanted, item.value.scalar().unwrap_or_default())?;
                continue;
            }
            // Array reading assigns both
            // the array text and that exact text followed by "^num".
            let text = if item.keyword_is("array") {
                item.value.scalar().unwrap_or_default()
            } else {
                item.key.as_deref().unwrap_or_default()
            };
            temps::assign(self, &mut array, text)?;
            let mut num = text.to_vec();
            num.extend_from_slice(b"^num");
            temps::assign(self, &mut count, &num)?;
            if !item.keyword_is("array") {
                temps::assign(self, &mut wanted, item.value.scalar().unwrap_or_default())?;
            }
        }
        let count = (self.raw(self.evaluate_operand(&count, scopes, None)?)? / ONE) as i32;
        let wanted = self.raw(self.evaluate_operand(&wanted, scopes, None)?)?;
        self.visit_indexed_values(&array, scopes, i64::from(count), |value| {
            Ok(self.raw(value)? == wanted)
        })
    }

    /// Evaluation's shared literal-index override; stop when the visitor matches.
    pub(super) fn visit_indexed_values(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        count: i64,
        mut visit: impl FnMut(Value) -> Result<bool, Failure>,
    ) -> Result<bool, Failure> {
        for at in 0..count.max(0) {
            let value = self.evaluate_operand(
                operand,
                scopes,
                Some(&ScopedVariable::literal(at.wrapping_mul(ONE))),
            )?;
            if visit(value)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// A value as the raw fixed5 the game holds; a scope object by its
    /// encoding.
    pub(super) fn raw(&self, value: Value) -> Result<i64, Failure> {
        match value {
            Value::Raw(raw) => Ok(raw),
            Value::Object(scope) => self.encode(scope),
        }
    }

    /// A value decoded as a scope.
    pub(super) fn decoded(&self, value: Value, scopes: Scopes) -> Result<Option<Scope>, Failure> {
        match value {
            // Encode the positive country out-parameter;
            // decode it with a clearing setter. The lazy identity
            // shortcut must not carry the original character into var:THIS.
            Value::Object(scope @ Scope::CountryLeader { .. }) => {
                self.decode(self.encode(scope)?, scopes)
            }
            Value::Object(scope) => Ok(Some(scope)),
            Value::Raw(raw) => self.decode(raw, scopes),
        }
    }

    /// `token:`: the token's id times
    /// 100000, through the recorded lexer.
    fn token_value(&self, token: &str) -> Result<Value, Failure> {
        match self.world().token_of(token.as_bytes())? {
            Some(id) => Ok(Value::Raw(i64::from(id).wrapping_mul(ONE))),
            None => Err(format!(
                "token:{token} names no recorded lexer token (FindExistingDynamicTokenFromString)"
            )
            .into()),
        }
    }

    /// The store path: the store, then a draw from its random
    /// state, then its built-in table, then the stored key, else the
    /// `?default` (0 without one). No store reads the `?default`.
    fn stored(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
        fallback: Option<&str>,
    ) -> Result<Value, Failure> {
        let store = self.resolve(operand, scopes, false)?.store;
        if let Some(store) = store {
            if operand.random {
                return self.next_variable_random(scopes).map(Value::Raw);
            }
            if variable_builtins::contains(store.builtins(), &operand.name) {
                if let Some(value) = self.builtin_variable(store, operand, scopes, index)? {
                    return Ok(Value::Raw(value));
                }
                return Err(crate::triggers::refusal::refused(format!(
                    "{} is a built-in game variable of {}, which the game computes before the stored keys; it is not reproduced",
                    String::from_utf8_lossy(&operand.name),
                    self.describe_store(store)
                )));
            }
        }
        if let Some(original) = fallback {
            self.admit_localisation_fallback_character(original)?;
        }
        if let Some(store) = store {
            let key = self.key(operand, scopes, index)?;
            if let Some(value) = self.world().variable(store, &key)? {
                return Ok(Value::Raw(value));
            }
        }
        match &operand.default {
            Some(default) => self.evaluate_operand(default, scopes, None),
            None => Ok(Value::Raw(0)),
        }
    }

    /// Builtin callback arguments: an index evaluated in
    /// the caller, and the target evaluated by its callback in the store's scope.
    fn builtin_variable(
        &self,
        store: Store,
        operand: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
    ) -> Result<Option<i64>, Failure> {
        let at = match index
            .or(operand.index.as_deref())
            .filter(|index| !index.num)
        {
            Some(index) => {
                let value = self.evaluate_operand(index, scopes, None)?;
                fixed_round(self.raw(value)?) as i32
            }
            None => -1,
        };
        if let Some(value) = self.map_builtin_array(store, &operand.name, at)? {
            return Ok(Some(value));
        }
        if let (
            Store::Country(slot),
            b"stability" | b"war_support" | b"has_stability" | b"has_war_support",
        ) = (store, operand.name.as_slice())
        {
            return self
                .country_stability_value(
                    slot,
                    matches!(operand.name.as_slice(), b"war_support" | b"has_war_support"),
                )
                .map(Some);
        }
        if let Some(value) = self.map_builtin_value(store, operand, scopes)? {
            return Ok(Some(value));
        }
        if let (Store::Country(slot), b"faction_leader") = (store, operand.name.as_slice()) {
            // Take member zero from the faction's stored roster,
            // then encode its tag.
            let value = match self.world().recorded().faction_leader(slot)? {
                Some(leader) => self.encode(Scope::Country(leader))?,
                None => 0,
            };
            return Ok(Some(value));
        }
        if let (Store::Country(slot), b"opinion") = (store, operand.name.as_slice()) {
            // Resolve the callback's country through its scoped operand.
            let callback = self.builtin_scope(store, operand, scopes)?;
            let target = match operand.target.as_deref() {
                Some(target) => {
                    let value = self.evaluate_operand(target, callback, None)?;
                    match self.decoded(value, callback)? {
                        Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => {
                            Some(slot)
                        }
                        Some(Scope::Combat) => Some(self.combat_country()?),
                        _ => None,
                    }
                }
                None => None,
            };
            return self.world().opinion(slot, target).map(Some);
        }
        self.world().builtin_variable(store, &operand.name)
    }

    pub(super) fn builtin_scope(
        &self,
        store: Store,
        operand: &ScopedVariable,
        scopes: Scopes,
    ) -> Result<Scopes, Failure> {
        let mut callback = scopes;
        if let Some(mut at) = scopes.cursor
            && self.event_graph.borrow().is_some()
        {
            let binding = self.event_graph.borrow();
            let graph = binding.as_ref().unwrap().borrow();
            for link in &operand.chain {
                let link = match link {
                    Link::This => continue,
                    Link::From => graph.scopes[at].from,
                    Link::Root => graph.scopes[at].root,
                    Link::Prev => graph.scopes[at].prev,
                };
                at = super::event_scopes::linked(&graph, at, link)?;
            }
            drop(graph);
            drop(binding);
            callback = self.event_scopes(at)?;
        } else if operand.chain.iter().any(|link| *link != Link::This) {
            callback = if self.walk(&operand.chain, scopes)? == Scope::Combat {
                Scopes::combat()
            } else {
                // The linked scope's own links are not described here.
                Scopes {
                    unknown: super::UnknownLinks {
                        root: true,
                        from: true,
                        prev: true,
                    },
                    ..scopes
                }
            };
        }
        // Setting country clears carried objects, keeping links.
        callback.this = match store {
            Store::Country(slot) => Scope::Country(slot),
            Store::State(state) => Scope::State(state),
            Store::Character(character) => Scope::Character(character),
            Store::Project { owner, id } => Scope::Project { owner, id },
            Store::IndustrialOrganisation(reference) => Scope::IndustrialOrganisation(reference),
            Store::Global => Scope::State(NULL_STATE),
        };
        callback.bare = true;
        Ok(callback)
    }

    fn describe_store(&self, store: Store) -> String {
        match store {
            Store::Global => "the global store".into(),
            Store::Country(slot) => self.describe(Scope::Country(slot)),
            Store::State(id) => format!("state {id}"),
            Store::Character(id) => format!("character {id}"),
            Store::Project { owner, id } => format!("project {owner}:{id}"),
            Store::IndustrialOrganisation(reference) => {
                self.describe(Scope::IndustrialOrganisation(reference))
            }
        }
    }

    /// Resolve storage: `global.`, a country specifier,
    /// an explicit state, an `a:` prefix's decoded scope, a saved event
    /// target, else the scope the chain walks to: a country's
    /// store, a state's, a unit leader's character's, a character's, a
    /// project's, an industrial organisation's, and with none of them the
    /// global store.
    fn resolve(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        identity_only: bool,
    ) -> Result<Resolved, Failure> {
        let at = |scope: Scope, store| Resolved {
            store: Some(store),
            object: Some(scope),
        };
        if operand.global {
            return Ok(Resolved {
                store: Some(Store::Global),
                object: None,
            });
        }
        if let Some(tag) = &operand.country {
            // Resolve the script country tag, then the country's store.
            return match self.resolve_tag(tag, scopes)? {
                Some(slot) => Ok(at(Scope::Country(slot), Store::Country(slot))),
                None => Err(format!(
                    "{tag} resolves to no country, whose store the game reads through a null country"
                ).into()),
            };
        }
        if let Some(state) = operand.state.filter(|state| *state >= 1) {
            // No existence test: a state the game lacks is a null read.
            let id = state as u32;
            if self.world().state_row(id).is_none() {
                return Err(format!(
                    "state {state} is not recorded, and the game reads a missing state's store through a null state"
                ).into());
            }
            return Ok(at(Scope::State(id), Store::State(id)));
        }
        if let Some(prefix) = &operand.prefix {
            let value = self.evaluate_operand(prefix, scopes, None)?;
            return Ok(match self.decoded(value, scopes)? {
                Some(scope) => self.scope_store(scope, identity_only)?,
                None => Resolved {
                    store: None,
                    object: None,
                },
            });
        }
        if operand.event_target.is_some() {
            let name = operand.event_target_name.as_deref().unwrap_or_default();
            return match self
                .saved_scope(name.as_bytes(), Reader::Variable, scopes)?
                .map(|s| s.this)
            {
                Some(scope) => self.scope_store(scope, identity_only),
                None => Ok(Resolved {
                    store: None,
                    object: None,
                }),
            };
        }
        let here = self.walk(&operand.chain, scopes)?;
        self.scope_store(here, identity_only)
    }

    /// The store of the scope the chain walked to.
    fn scope_store(&self, scope: Scope, identity_only: bool) -> Result<Resolved, Failure> {
        let at = |store| Resolved {
            store: Some(store),
            object: Some(scope),
        };
        if identity_only
            && matches!(
                scope,
                Scope::IndustrialOrganisation(_) | Scope::Project { .. } | Scope::Raid(_)
            )
        {
            // Encoding reads the out-object identity, not stored variable keys.
            return Ok(Resolved {
                store: None,
                object: Some(scope),
            });
        }
        Ok(match scope {
            Scope::Country(slot) => at(Store::Country(slot)),
            Scope::CountryLeader { country, .. } if self.variable_scope_has_country(country)? => {
                at(Store::Country(country))
            }
            Scope::Combat => {
                let slot = self.combat_country()?;
                Resolved {
                    store: Some(Store::Country(slot)),
                    object: Some(Scope::Country(slot)),
                }
            }
            // The null state holds nothing: the global store.
            Scope::State(NULL_STATE) => Resolved {
                store: Some(Store::Global),
                object: None,
            },
            // A state the game lacks is logged and has
            // no store.
            Scope::State(id) if self.world().state_row(id).is_none() => Resolved {
                store: None,
                object: Some(scope),
            },
            Scope::State(id) => at(Store::State(id)),
            Scope::Character(id) => at(Store::Character(id)),
            // The organisation's own persistent store.
            Scope::IndustrialOrganisation(reference) => {
                at(Store::IndustrialOrganisation(reference))
            }
            Scope::Project { owner, id } => at(Store::Project { owner, id }),
            Scope::Raid(reference) => {
                return Err(format!("raid {reference:#x} variable store is not recorded").into());
            }
            // Storage resolution does not examine UNIT.
            Scope::Unit(_) => Resolved {
                store: Some(Store::Global),
                object: None,
            },
            // Use the unit leader's character;
            // the branch sets no character out-parameter.
            Scope::Leader(reference)
            | Scope::CountryLeader {
                leader: reference, ..
            } => {
                self.core()?;
                let row = self
                    .world()
                    .unit_leader(reference)?
                    .ok_or_else(|| -> Failure {
                        format!("unit leader {reference:#x} is not recorded").into()
                    })?;
                Resolved {
                    store: Some(Store::Character(row.character_id)),
                    object: None,
                }
            }
            Scope::Province(province) => {
                return Err(format!(
                    "which store province {province}'s scope reads is not reproduced"
                )
                .into());
            }
        })
    }

    /// Follow FROM, ROOT and PREV in order from THIS; THIS stays.
    /// A link read from a linked scope is its own, which only the combat
    /// scope (whose links are itself) describes here.
    fn walk(&self, chain: &[Link], scopes: Scopes) -> Result<Scope, Failure> {
        if let Some(at) = scopes.cursor
            && let Some(graph) = self.event_graph.borrow().as_ref()
        {
            let graph = graph.borrow();
            let mut at = at;
            for link in chain {
                let link = match link {
                    Link::This => continue,
                    Link::From => graph.scopes[at].from,
                    Link::Root => graph.scopes[at].root,
                    Link::Prev => graph.scopes[at].prev,
                };
                at = super::event_scopes::linked(&graph, at, link)?;
            }
            return Ok(super::event_scopes::object(graph.scopes[at].this));
        }
        let mut here = scopes.this;
        let mut moved = false;
        for link in chain {
            let keyword = match link {
                Link::This => continue,
                Link::From => ScopeKeyword::From,
                Link::Root => ScopeKeyword::Root,
                Link::Prev => ScopeKeyword::Prev,
            };
            if moved && here != Scope::Combat {
                return Err(format!(
                    "{} of a linked scope is that scope's own link, which is not known here",
                    keyword.name().to_ascii_uppercase()
                )
                .into());
            }
            here = scopes.link(keyword)?.ok_or_else(|| -> Failure {
                format!(
                    "this scope has no {}, which the variable's scope chain follows",
                    keyword.name().to_ascii_uppercase()
                )
                .into()
            })?;
            moved = true;
        }
        Ok(here)
    }

    /// Build the key: the name, the `@` target's text (a country's
    /// lower-case tag, a state's id, else the value's whole part and, when
    /// its fraction is not zero, `_` and the fraction), and the `^` index
    /// (`^num`, or `^` and the rounded index).
    fn key(
        &self,
        operand: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
    ) -> Result<Vec<u8>, Failure> {
        let mut key = operand.name.clone();
        if let Some(target) = &operand.target {
            let value = self.evaluate_operand(target, scopes, None)?;
            match self.decoded(value, scopes)? {
                Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => {
                    key.extend(self.tag_text(slot)?)
                }
                Some(Scope::Combat) => key.extend(self.tag_text(self.combat_country()?)?),
                Some(Scope::State(id)) => key.extend(id.to_string().into_bytes()),
                _ => {
                    // Numeric suffixes evaluate the target again.
                    // Alias conditions can change temps between the reads.
                    let value = self.evaluate_operand(target, scopes, None)?;
                    let raw = self.raw(value)?;
                    let fraction =
                        (raw as i32).wrapping_sub(((raw / ONE) as i32).wrapping_mul(100_000));
                    key.extend(((raw / ONE) as u32).to_string().into_bytes());
                    if fraction != 0 {
                        key.push(b'_');
                        key.extend((fraction as u32).to_string().into_bytes());
                    }
                }
            }
        }
        if let Some(index) = index.or(operand.index.as_deref()) {
            if index.num {
                key.extend(b"^num");
            } else {
                let value = self.evaluate_operand(index, scopes, None)?;
                let at = fixed_round(self.raw(value)?) as i32;
                key.push(b'^');
                key.extend(at.to_string().into_bytes());
            }
        }
        Ok(key)
    }

    /// A country's stored tag spelling, lower-cased.
    fn tag_text(&self, slot: u32) -> Result<Vec<u8>, Failure> {
        self.world()
            .tag(slot)
            .map(<[u8]>::to_ascii_lowercase)
            .ok_or_else(|| -> Failure { format!("country slot {slot} has no recorded tag").into() })
    }

    /// The thread-local store at
    /// `key`: a temp the caller injected, else nothing in a fresh context;
    /// what the evaluator cannot know is refused.
    fn temp(&self, key: &[u8], scopes: Scopes) -> Result<Option<Value>, Failure> {
        if let Some(value) = self.local_temps.borrow().get(key) {
            if let Err(failure) = value {
                failure.reached();
            }
            return value.clone().map(Some);
        }
        let injected = match key {
            b"target_state" => scopes
                .target_state
                .map(|state| Value::Object(Scope::State(state))),
            b"target_province" => scopes
                .target_province
                .map(|province| Value::Raw(i64::from(province).wrapping_mul(ONE))),
            b"target_leader" => scopes
                .target_leader
                .map(|leader| Value::Object(Scope::Leader(leader))),
            _ => None,
        };
        let shown = String::from_utf8_lossy(key);
        // An unreproduced writer may overwrite even an injected temp.
        if let Some(writer) = self.temps_written() {
            writer.reached();
            return Err(writer.context(|writer| format!(
                "{shown} may be a temp that {writer}, reached earlier in this evaluation, wrote; that temp write is not reproduced"
            )));
        }
        if self.temps() == Temps::Inherited {
            return Err(inherited_temps_failure());
        }
        if injected.is_some() {
            return Ok(injected);
        }
        if key == b"random_seed" {
            if self.checking.get() > 0 || self.description_depth.get() > 0 {
                self.initialize_variable_random(scopes);
                let values = self.local_temps.borrow();
                let value = values.get(key).unwrap();
                if let Err(failure) = value {
                    failure.reached();
                }
                return value.clone().map(Some);
            }
            return Err(format!(
                "{shown} is a residual TLS seed outside an active context, which is not recorded"
            )
            .into());
        }
        match self.temps() {
            Temps::Fresh => Ok(None),
            Temps::Inherited => Err(inherited_temps_failure()),
            Temps::Stale => Err(format!(
                "{shown} is read from the thread's temps first, which outside a script context keep what its last context set; they are not recorded"
            ).into()),
        }
    }

    /// Decode an encoded object, or a
    /// whole number that is an existing state's id.
    fn decode(&self, raw: i64, scopes: Scopes) -> Result<Option<Scope>, Failure> {
        if raw == 0 {
            return Ok(None);
        }
        let low = (raw as u64 & 0x0fff_ffff) as u32;
        let world = self.world();
        let object = |kind: &str| {
            Err(format!(
                "the value {raw:#x} encodes {kind}, a scope the evaluator does not reproduce"
            )
            .into())
        };
        match raw as u64 & 0xffff_ffff_f000_0000 {
            0x4000_0000 => {
                // Resolve the low three bytes as a NUL-terminated country
                // specifier.
                let bytes = (raw as u32 & 0x00ff_ffff).to_le_bytes();
                let tag: Vec<u8> = bytes.iter().copied().take_while(|b| *b != 0).collect();
                if let Ok(tag) = std::str::from_utf8(&tag)
                    && Names::is_country(world.recorded(), tag)
                {
                    return Ok(self.resolve_tag(tag, scopes)?.map(Scope::Country));
                }
            }
            0xffff_ffff_c000_0000 => {
                // A hole in the state list decodes to no state.
                let state = world.state_of_value(raw)?;
                return Ok((state != NULL_STATE).then_some(Scope::State(state)));
            }
            0x2000_0000 => {
                return Ok(world
                    .character_exists(low)?
                    .then_some(Scope::Character(low)));
            }
            0xffff_ffff_8000_0000 => return object("a unit leader"),
            0x1000_0000 => {
                return self
                    .decoded_carrier(engine_projects::nproject_cproject::REFERENCE_TYPE, low);
            }
            0x3000_0000 => return object("a faction"),
            0x6000_0000 => return self.decoded_carrier(79, low),
            0xffff_ffff_a000_0000 => return object("a purchase contract"),
            0xffff_ffff_e000_0000 => {
                return self
                    .decoded_carrier(engine_native::craid_instance::REFERENCE_TYPE_VALUE, low);
            }
            _ => {}
        }
        // A whole number whose state exists.
        if raw > 0 && raw % ONE == 0 {
            let state = world.state_of_value(raw)?;
            if state != NULL_STATE {
                return Ok(Some(Scope::State(state)));
            }
        }
        Ok(None)
    }

    /// Scope object encoding: a country's tag bytes or a state's id,
    /// with the matching object discriminator.
    pub(super) fn encode(&self, scope: Scope) -> Result<i64, Failure> {
        let slot = match scope {
            Scope::State(NULL_STATE) => return Ok(0),
            Scope::State(id) => return Ok((0xffff_ffff_c000_0000_u64 | u64::from(id)) as i64),
            Scope::Project { owner, id } => {
                self.world().project(owner, id)?;
                return Ok(i64::from(id | 0x1000_0000));
            }
            Scope::IndustrialOrganisation(reference) => {
                self.world().industrial_organisation(reference)?;
                return Ok(i64::from((reference >> 32) as u32 | 0x6000_0000));
            }
            Scope::Raid(reference) => {
                self.world().raid(reference)?;
                return Ok((0xffff_ffff_e000_0000_u64 | (reference >> 32)) as i64);
            }
            Scope::Country(slot) => slot,
            Scope::CountryLeader { country, .. } if self.variable_scope_has_country(country)? => {
                country
            }
            Scope::Combat => self.combat_country()?,
            Scope::Character(id) => {
                return Err(format!(
                    "the value of character {id}'s scope is its unit leader's when it has one, which is not reproduced"
                ).into());
            }
            other => {
                return Err(format!(
                    "the value of {}'s scope is not reproduced",
                    self.describe(other)
                )
                .into());
            }
        };
        let tag = if let Some(tag) = self.world().tag(slot) {
            tag
        } else if slot == 0 {
            super::inputs::table(
                self.world().tables(),
                replay_data::rows::TableId::ReservedCountryNaming,
            );
            const TAG: usize =
                replay_data::rows::column::<replay_data::rows::ReservedCountryNamingRow>("tag");
            let reserved = super::world::table::<replay_data::rows::tables::ReservedCountryNaming>(
                self.world().tables(),
                0,
            )?;
            if reserved.len() != 1 {
                return Err("reserved country naming does not hold one reserved country".into());
            }
            // The one reserved row is keyed zero.
            reserved
                .text(0, |row| &row.tag, TAG)
                .map_err(|why| {
                    super::refusal::input_error(
                        replay_data::rows::TableId::ReservedCountryNaming,
                        why,
                    )
                })?
                .ok_or("reserved country naming's one row is not keyed zero")?
        } else {
            return Err(format!("country slot {slot} has no recorded tag").into());
        };
        let mut bytes = [0_u8; 4];
        for (at, byte) in tag.iter().take(3).enumerate() {
            bytes[at] = *byte;
        }
        Ok(i64::from(u32::from_le_bytes(bytes) | 0x4000_0000))
    }
}

/// A `check_variable` compare token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compare {
    Equals,
    NotEquals,
    Less,
    LessOrEquals,
    Greater,
    GreaterOrEquals,
    /// A token the reader reports but keeps: evaluation asserts an invalid
    /// comparison and is false.
    Invalid,
}

impl Evaluator<'_, '_> {
    /// Read a variable comparison:
    /// `var` and `value` parse their value tokens,
    /// `compare` keeps the value's token (default
    /// `greater_than_or_equals`), `tooltip` is text, and any other key is the
    /// shorthand: the key text is the var side, the value text the value
    /// side, and `=`, `>` and `<` set equals, greater and less. A side never
    /// set is the initial literal 0. The value side is
    /// evaluated first, then the var side, both in the trigger's scope, and
    /// compared as signed fixed5. A value that is not a block reads no key
    /// and is reported: 0 >= 0, true.
    pub(super) fn check_variable(&self, item: &Item, scopes: Scopes) -> Result<bool, Failure> {
        let Some(fields) = item.value.items() else {
            return Ok(true);
        };
        let mut var: Option<&[u8]> = None;
        let mut value: Option<&[u8]> = None;
        let mut compare = Compare::GreaterOrEquals;
        // A second parse into the same operand keeps what the first left;
        // it does not reset every field.
        fn set<'t>(side: &mut Option<&'t [u8]>, text: &'t [u8]) -> Result<(), Failure> {
            if side.replace(text).is_some() {
                return Err(
                    "a side of the comparison is read twice into one operand, whose fields the second read does not all reset"
                        .into(),);
            }
            Ok(())
        }
        for field in fields {
            let text = field.value.scalar().unwrap_or_default();
            match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                Some(b"tooltip") => {}
                Some(b"var") => set(&mut var, text)?,
                Some(b"value") => set(&mut value, text)?,
                Some(b"compare") => {
                    compare = match field.value_keyword().as_deref() {
                        Some("equals") => Compare::Equals,
                        Some("not_equals") => Compare::NotEquals,
                        Some("less_than") => Compare::Less,
                        Some("less_than_or_equals") => Compare::LessOrEquals,
                        Some("greater_than") => Compare::Greater,
                        Some("greater_than_or_equals") => Compare::GreaterOrEquals,
                        _ => Compare::Invalid,
                    };
                }
                _ => {
                    set(&mut var, field.key.as_deref().unwrap_or_default())?;
                    compare = match field.operator {
                        Operator::Equals => Compare::Equals,
                        Operator::Greater => Compare::Greater,
                        Operator::Less => Compare::Less,
                    };
                    set(&mut value, text)?;
                }
            }
        }
        let side = |text: Option<&[u8]>| match text {
            Some(text) => self.scoped_value(text, scopes),
            None => Ok(0),
        };
        let right = side(value)?;
        let left = side(var)?;
        Ok(match compare {
            Compare::Equals => left == right,
            Compare::NotEquals => left != right,
            Compare::Less => left < right,
            Compare::LessOrEquals => left <= right,
            Compare::Greater => left > right,
            Compare::GreaterOrEquals => left >= right,
            Compare::Invalid => false,
        })
    }

    /// Read the value token as one operand (a literal is reported and stays
    /// a literal, which is present), then [`Self::exists`].
    /// A block is no form
    /// its reader takes.
    pub(super) fn has_variable(&self, item: &Item, scopes: Scopes) -> Result<bool, Failure> {
        let text = item
            .value
            .scalar()
            .ok_or("the operand is a block, which the reader does not take")?;
        let operand = self.operand(text)?;
        self.exists(&operand, scopes)
            .map_err(|why| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text))))
    }
}

/// The operand forms refused before evaluating, by name.
fn unported(operand: &ScopedVariable) -> Option<String> {
    if !operand.parsed {
        return Some(
            "the operand is empty: the scoped-variable reader logs it and sets nothing".into(),
        );
    }
    if let Some(why) = &operand.unset {
        return Some(format!(
            "{why}; the operand then evaluates as its scope, which is not reproduced"
        ));
    }
    if operand.ai_random {
        return Some("ai_random is an AI random draw, which is not reproduced".into());
    }
    if let Some(name) = &operand.mtth {
        return Some(format!(
            "mtth:{name} evaluates a mean time database entry, which is not read"
        ));
    }
    None
}
