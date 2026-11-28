use crate::triggers::Failure;

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

use content_reader_parser::parser::{Item, Operator};
use content_reader_script::{scope::{Keyword as ScopeKeyword, Names}, scoped_variable::{self, Constant, Link, ScopedVariable}, variable_builtins};

use super::{
    Scope, Scopes,
    eval::Evaluator,
    saved_event_targets::Reader,
    world::{NULL_STATE, fixed_round},
};
use crate::state_resources::ONE;

mod random;
pub(super) mod temps;

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Temps {
    #[default]
    Fresh,
    Stale,
    Inherited,
}

pub(super) fn inherited_temps_failure() -> Failure {
    "force_update_map_mode caller temporary variables are not recorded".into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Store {
    Global,
    Country(u32),
    State(u32),
    Character(u32),
    Project {
        owner: u32,
        id: u32,
    },
    IndustrialOrganisation(u64),
}

impl Store {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Raw(i64),
    Object(Scope),
}

struct Resolved {
    store: Option<Store>,
    object: Option<Scope>,
}

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

    pub fn scoped_value(&self, text: &[u8], scopes: Scopes) -> Result<i64, Failure> {
        let within =
            |why: Failure| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text)));
        let operand = self.operand(text).map_err(within)?;
        self.evaluate_operand(&operand, scopes, None)
            .and_then(|value| self.raw(value))
            .map_err(within)
    }

    pub const NULL_IDEOLOGY: &'static str =
        "ideology database null-object identity is unrecorded";

    pub fn ideology_specifier(
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
        let constant = content_reader_parser::fixed5::fixed5_to_string(value, 5);
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
        let mut previous_owner = None;
        for record in active.iter().chain(retired.iter()) {
            let record = record.map_err(|why| {
                super::refusal::input_error(replay_data::rows::TableId::CountryCharacters, why)
            })?;
            let Some(id) = record.character else {
                continue;
            };
            let owner = if let Some((raw, owner)) = previous_owner
                && raw == record.country
            {
                owner
            } else {
                let owner = self.world().slot(record.country).ok_or_else(|| {
                    self.world().unlinked("country characters/country retired characters", record.country)
                })?;
                previous_owner = Some((record.country, owner));
                owner
            };
            if owner != country {
                continue;
            }
            let frame = self.world().tables();
            localisation_status(
                frame,
                replay_data::rows::TableId::Characters,
                frame.status(replay_data::rows::TableId::Characters),
            )?;
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

    pub fn scoped_exists(&self, text: &[u8], scopes: Scopes) -> Result<bool, Failure> {
        self.exists(&self.operand(text)?, scopes)
    }

    pub fn scoped_scope(&self, text: &[u8], scopes: Scopes) -> Result<Option<Scope>, Failure> {
        let within =
            |why: Failure| why.context(|why| format!("{}: {why}", String::from_utf8_lossy(text)));
        let operand = self.operand(text).map_err(within)?;
        let value = self
            .evaluate_operand(&operand, scopes, None)
            .map_err(within)?;
        self.decoded(value, scopes).map_err(within)
    }

    pub fn scoped_country(&self, text: &[u8], scopes: Scopes) -> Result<Option<u32>, Failure> {
        Ok(match self.scoped_scope(text, scopes)? {
            Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => Some(slot),
            Some(Scope::Combat) => Some(self.combat_country()?),
            _ => None,
        })
    }

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
        if let Some(value) = operand.literal {
            return Ok(Value::Raw(value));
        }
        if let Some(why) = unported(operand) {
            return Err(why.into());
        }
        if let Some(token) = &operand.token {
            return self.token_value(token);
        }
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
            None if operand.temp => Ok(Value::Raw(0)),
            None => self.stored(operand, scopes, index, fallback),
        }
    }

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

    pub(super) fn is_in_array(&self, block: &[Item], scopes: Scopes) -> Result<bool, Failure> {
        let (mut array, mut count, mut wanted) = (temps::zero(), temps::zero(), temps::zero());
        for item in block {
            if item.keyword_is("size") || item.keyword_is("index") {
                continue;
            }
            if item.keyword_is("value") {
                temps::assign(self, &mut wanted, item.value.scalar().unwrap_or_default())?;
                continue;
            }
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

    pub(super) fn raw(&self, value: Value) -> Result<i64, Failure> {
        match value {
            Value::Raw(raw) => Ok(raw),
            Value::Object(scope) => self.encode(scope),
        }
    }

    pub(super) fn decoded(&self, value: Value, scopes: Scopes) -> Result<Option<Scope>, Failure> {
        match value {
            Value::Object(scope @ Scope::CountryLeader { .. }) => {
                self.decode(self.encode(scope)?, scopes)
            }
            Value::Object(scope) => Ok(Some(scope)),
            Value::Raw(raw) => self.decode(raw, scopes),
        }
    }

    fn token_value(&self, token: &str) -> Result<Value, Failure> {
        match self.world().token_of(token.as_bytes())? {
            Some(id) => Ok(Value::Raw(i64::from(id).wrapping_mul(ONE))),
            None => Err(format!(
                "token:{token} names no recorded lexer token (FindExistingDynamicTokenFromString)"
            )
            .into()),
        }
    }

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
            let value = match self.world().recorded().faction_leader(slot)? {
                Some(leader) => self.encode(Scope::Country(leader))?,
                None => 0,
            };
            return Ok(Some(value));
        }
        if let (Store::Country(slot), b"opinion") = (store, operand.name.as_slice()) {
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
            return match self.resolve_tag(tag, scopes)? {
                Some(slot) => Ok(at(Scope::Country(slot), Store::Country(slot))),
                None => Err(format!(
                    "{tag} resolves to no country, whose store the game reads through a null country"
                ).into()),
            };
        }
        if let Some(state) = operand.state.filter(|state| *state >= 1) {
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
            Scope::State(NULL_STATE) => Resolved {
                store: Some(Store::Global),
                object: None,
            },
            Scope::State(id) if self.world().state_row(id).is_none() => Resolved {
                store: None,
                object: Some(scope),
            },
            Scope::State(id) => at(Store::State(id)),
            Scope::Character(id) => at(Store::Character(id)),
            Scope::IndustrialOrganisation(reference) => {
                at(Store::IndustrialOrganisation(reference))
            }
            Scope::Project { owner, id } => at(Store::Project { owner, id }),
            Scope::Raid(reference) => {
                return Err(format!("raid {reference:#x} variable store is not recorded").into());
            }
            Scope::Unit(_) => Resolved {
                store: Some(Store::Global),
                object: None,
            },
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

    fn tag_text(&self, slot: u32) -> Result<Vec<u8>, Failure> {
        self.world()
            .tag(slot)
            .map(<[u8]>::to_ascii_lowercase)
            .ok_or_else(|| -> Failure { format!("country slot {slot} has no recorded tag").into() })
    }

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
                let bytes = (raw as u32 & 0x00ff_ffff).to_le_bytes();
                let tag: Vec<u8> = bytes.iter().copied().take_while(|b| *b != 0).collect();
                if let Ok(tag) = std::str::from_utf8(&tag)
                    && Names::is_country(world.recorded(), tag)
                {
                    return Ok(self.resolve_tag(tag, scopes)?.map(Scope::Country));
                }
            }
            0xffff_ffff_c000_0000 => {
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
                    .decoded_carrier(engine_native_air::craid_instance::REFERENCE_TYPE_VALUE, low);
            }
            _ => {}
        }
        if raw > 0 && raw % ONE == 0 {
            let state = world.state_of_value(raw)?;
            if state != NULL_STATE {
                return Ok(Some(Scope::State(state)));
            }
        }
        Ok(None)
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compare {
    Equals,
    NotEquals,
    Less,
    LessOrEquals,
    Greater,
    GreaterOrEquals,
    Invalid,
}

impl Evaluator<'_, '_> {
    pub(super) fn check_variable(&self, item: &Item, scopes: Scopes) -> Result<bool, Failure> {
        let Some(fields) = item.value.items() else {
            return Ok(true);
        };
        let mut var: Option<&[u8]> = None;
        let mut value: Option<&[u8]> = None;
        let mut compare = Compare::GreaterOrEquals;
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
