//! Hourly raw combat window operands. Fixed-point stays integer; handles stay complete.
use super::*;
use crate::value::value_struct;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CombatModifierValue {
    pub id: u32,
    pub value_a: i64,
    pub value_b: i64,
}
value_struct!(CombatModifierValue {
    id: u32,
    value_a: i64,
    value_b: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatCommanderTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub kind: u32,
    pub relevant: u8,
    pub token: Vec<u8>,
}
value_struct!(LandCombatCommanderTrait { token_id: i32, definition_ordinal: i32, kind: u32, relevant: u8, token: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatCommanderLearningTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub required_experience: i32,
    pub experience: i64,
    pub token: Vec<u8>,
}
value_struct!(LandCombatCommanderLearningTrait { token_id: i32, definition_ordinal: i32, required_experience: i32, experience: i64, token: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatCommanderPortraitModifier {
    pub source: u8,
    pub id: u32,
    pub value: i64,
    pub metadata_flags: u32,
    pub category_mask: u32,
    pub feature_selector: u32,
}
value_struct!(LandCombatCommanderPortraitModifier {
    source: u8,
    id: u32,
    value: i64,
    metadata_flags: u32,
    category_mask: u32,
    feature_selector: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatCommanderPortraitTooltip {
    pub source: u8,
    pub bucket: u32,
    pub key: u32,
    pub text: Vec<u8>,
}
value_struct!(LandCombatCommanderPortraitTooltip { source: u8, bucket: u32, key: u32, text: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CombatCommanderSkillDefinition {
    pub present: u8,
    pub valid: u8,
    pub level: i32,
    pub required_experience: i32,
    pub category: i32,
}
value_struct!(CombatCommanderSkillDefinition {
    present: u8,
    valid: u8,
    level: i32,
    required_experience: i32,
    category: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CombatCommanderPortraitEntry {
    pub role: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub text: Vec<u8>,
}
value_struct!(CombatCommanderPortraitEntry { role: u32, size: u32, is_set: u8, kind: u32, text: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct CombatCommanderPresentation {
    pub character: u64,
    pub character_state: u8,
    pub character_country: u32,
    pub character_gender: u32,
    pub character_role_leader: u64,
    pub character_fallback_leader: u64,
    pub character_role_source: u8,
    pub fallback_text: [Vec<u8>; 3],
    pub portraits: Vec<CombatCommanderPortraitEntry>,
    pub role_traits: Vec<LandCombatCommanderTrait>,
}
value_struct!(CombatCommanderPresentation { character: u64, character_state: u8, character_country: u32, character_gender: u32, character_role_leader: u64, character_fallback_leader: u64, character_role_source: u8, fallback_text: [Vec<u8>;3], portraits: Vec<CombatCommanderPortraitEntry>, role_traits: Vec<LandCombatCommanderTrait> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCommanderInputs {
    pub commander: u64,
    pub country: u32,
    pub category: i32,
    pub experience: i64,
    pub skills: [i32; 5],
    pub skill_definitions: [CombatCommanderSkillDefinition; 5],
    pub next_skill: CombatCommanderSkillDefinition,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub presentation: CombatCommanderPresentation,
    pub traits: Vec<LandCombatCommanderTrait>,
    pub learning_traits: Vec<LandCombatCommanderLearningTrait>,
    pub learning_threshold: i64,
    pub modifiers: Vec<LandCombatCommanderPortraitModifier>,
    pub modifier_feature_flags: u32,
    pub modifier_callback_kind: u8,
    pub tooltips: Vec<LandCombatCommanderPortraitTooltip>,
    /// Stored timed-trait inputs, updated when traits are added and daily.
    /// NotRecorded in recordings predating this input.
    pub timed_traits: Recorded<Vec<LandCombatCommanderTimedTrait>>,
}
value_struct!(NavalCommanderInputs { commander: u64, country: u32, category: i32, experience: i64, skills: [i32; 5], skill_definitions: [CombatCommanderSkillDefinition;5], next_skill: CombatCommanderSkillDefinition, name: Vec<u8>, description: Vec<u8>, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait>, learning_threshold: i64, modifiers: Vec<LandCombatCommanderPortraitModifier>, modifier_feature_flags: u32, modifier_callback_kind: u8, tooltips: Vec<LandCombatCommanderPortraitTooltip> } appended NavalCommanderInputs { timed_traits: Recorded<Vec<LandCombatCommanderTimedTrait>> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatSideDetails {
    pub committed: Vec<u64>,
    pub reserves: Vec<u64>,
    pub tactic_id: i32,
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    pub tactic_token: Vec<u8>,
    pub air_counts: Vec<i32>,
    /// The combat side's CAS hover anti-air cache, which [`LandCombatSideInputs::anti_air_attack`]
    /// records; formats 101 and earlier hold it here ([`LandCombatDetailsRow::cas_inputs`]).
    pub anti_air_attack: RetiredKept<i64>,
    pub air_kills: u32,
    /// The round's air strength and organisation damage on the combat side, the window's
    /// LAND_COMBAT_CAS_INFO line.
    pub damage: [i64; 2],
    /// The ground strength and organisation damage lanes: only save loading, round resolution and
    /// save serialization touch them, so no display reads them.
    pub ground_damage: RetiredNested<[i64; 2]>,
    /// The prevented CAS damage lanes, which [`LandCombatSideInputs::prevented_damage`] records;
    /// formats 101 and earlier hold them here, exact.
    pub prevented_damage: RetiredKept<[i64; 2]>,
    /// The sum of the 24 organisation loss samples the combat side holds once
    /// `organisation_loss_count` reaches 24, and 0 before, when the samples are not read
    /// ([`Self::loss_sum`]). The marker's survival estimate is the only reader that is not the save
    /// or the round itself, and it takes only this sum.
    pub organisation_loss_sum: i64,
    pub organisation_loss_count: u32,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    /// Raw unit leader experience, read only by the commander skill hover.
    pub commander_experience: RetiredNested<i64>,
    /// Raw saved name token (the unit leader's name); the viewer localises it.
    pub commander_name_token: Vec<u8>,
    /// The description token, read only by the portrait hover.
    pub commander_description_token: RetiredNested<Vec<u8>>,
    /// The army leader's formation reference of a category 0/1 commander, for the HQ badge; zero
    /// otherwise.
    pub commander_formation: u64,
    pub presentation: CombatCommanderPresentation,
    pub traits: Vec<LandCombatCommanderTrait>,
    /// Timed traits (the unit leader's timed-trait vector), read only by trait hovers.
    pub timed_traits: RetiredNested<Vec<LandCombatCommanderTimedTrait>>,
    /// Learning traits, read only by the commander hover.
    pub learning_traits: RetiredNested<Vec<LandCombatCommanderLearningTrait>>,
}
impl LandCombatSideDetails {
    /// The organisation loss samples a side holds after its 24th round (`ORGANISATION_LOSS_WINDOW`)
    /// as the one figure any displayed reader takes from them: their sum. The survival estimate
    /// loads slot 0 and adds the other 23 with wrapping 64-bit addition. The other readers are save
    /// serialization, save loading and round resolution.
    pub fn loss_sum(samples: &[i64; 24]) -> i64 {
        samples
            .iter()
            .fold(0_i64, |sum, value| sum.wrapping_add(*value))
    }
}

/// The largest committed division count a side that stores its loss sum can
/// mark ([`encode_committed`]).
const MARKED_COMMITTED: u32 = u32::MAX / 2;

/// A side's committed divisions, the first field of its encoding. A side
/// that stores its organisation loss sum (`marked`) stores the list's
/// element count inverted (`!count`); the test-only historical writer,
/// standing in for format 101, stores the count itself. A stored count is at
/// most the payload's bytes over eight, its elements' width, so below 2^31,
/// and a marked one above, so from
/// [`crate::rows::compatibility::LOSS_SUM_FORMAT`] the reader tells format-102
/// captures that hold the 24 samples from current ones ([`decode_committed`]).
/// A reader that knows only the samples refuses a marked count as longer than
/// its payload.
fn encode_committed(divisions: &[u64], marked: bool, output: &mut Vec<u8>) -> io::Result<()> {
    let count = u32::try_from(divisions.len())
        .map_err(|_| invalid("nested collection count exceeds u32"))?;
    if !marked {
        output.extend_from_slice(&count.to_le_bytes());
    } else if count <= MARKED_COMMITTED {
        output.extend_from_slice(&(!count).to_le_bytes());
    } else {
        return Err(invalid("committed division count exceeds the marked range"));
    }
    <u64 as Value>::encode_many(divisions, output)
}

/// A side's committed divisions, and whether the side stores its loss sum
/// rather than the 24 samples ([`encode_committed`]).
fn decode_committed(input: &mut Decoder<'_>) -> io::Result<(Vec<u64>, bool)> {
    if input.format < crate::rows::compatibility::LOSS_SUM_FORMAT {
        return Ok((Value::decode(input)?, false));
    }
    let stored = input.u32()?;
    let (count, marked) = if stored > MARKED_COMMITTED {
        (!stored, true)
    } else {
        (stored, false)
    };
    Ok((<u64 as Value>::decode_many(count as usize, input)?, marked))
}

/// [`LandCombatSideDetails`]' [`Value`] impl: the
/// [`crate::value::value_struct`] layout, its fields in declaration order,
/// except that current writers mark the committed list ([`encode_committed`])
/// and store the organisation loss sum, one i64, where older layouts hold the
/// 24 samples it sums. `before` lists the fields between the two, `after`
/// the fields after the loss. No field takes a display step.
macro_rules! side_value {
    ([$($before:ident: $before_ty:ty),* $(,)?], [$($after:ident: $after_ty:ty),* $(,)?]) => {
        impl Value for LandCombatSideDetails {
            fn heap_bytes(&self) -> usize {
                <Vec<u64> as Value>::heap_bytes(&self.committed)
                    $(+ <$before_ty as Value>::heap_bytes(&self.$before))*
                    + <i64 as Value>::heap_bytes(&self.organisation_loss_sum)
                    $(+ <$after_ty as Value>::heap_bytes(&self.$after))*
            }
            #[cfg(any(test, feature = "test-support"))]
            const STRUCT_FIELDS: usize =
                2 + [$(stringify!($before)),*].len() + [$(stringify!($after)),*].len();
            const MINIMUM_ENCODED_BYTES: usize = <Vec<u64> as Value>::MINIMUM_ENCODED_BYTES
                $(.saturating_add(<$before_ty as Value>::MINIMUM_ENCODED_BYTES))*
                .saturating_add(<i64 as Value>::MINIMUM_ENCODED_BYTES)
                $(.saturating_add(<$after_ty as Value>::MINIMUM_ENCODED_BYTES))*;
            const HAS_STEPS: bool = crate::value::any(&[
                $(<$before_ty as Value>::HAS_STEPS,)*
                $(<$after_ty as Value>::HAS_STEPS,)*
            ]);

            fn round_display(&mut self) {
                $(crate::value::round_field(&mut self.$before, None);)*
                $(crate::value::round_field(&mut self.$after, None);)*
            }
            fn recorded_bytes(&self) -> io::Result<usize> {
                let size = Value::recorded_bytes(&self.committed)?;
                $(let size = allocation::add(size, Value::recorded_bytes(&self.$before)?)?;)*
                let size = allocation::add(size, loss_bytes())?;
                $(let size = allocation::add(size, Value::recorded_bytes(&self.$after)?)?;)*
                Ok(size)
            }
            fn encoded_bytes(&self) -> io::Result<usize> {
                let size = Value::encoded_bytes(&self.committed)?;
                $(let size = allocation::add(size, Value::encoded_bytes(&self.$before)?)?;)*
                let size = allocation::add(size, loss_bytes())?;
                $(let size = allocation::add(size, Value::encoded_bytes(&self.$after)?)?;)*
                Ok(size)
            }
            fn try_clone(&self) -> io::Result<Self> {
                Ok(Self {
                    committed: Value::try_clone(&self.committed)?,
                    $($before: Value::try_clone(&self.$before)?,)*
                    organisation_loss_sum: self.organisation_loss_sum,
                    $($after: Value::try_clone(&self.$after)?,)*
                })
            }
            fn encode_reserved(&self, output: &mut Vec<u8>) -> io::Result<()> {
                let marked = historical_stamp().is_none();
                encode_committed(&self.committed, marked, output)?;
                $(self.$before.encode_reserved(output)?;)*
                if marked {
                    self.organisation_loss_sum.encode_reserved(output)?;
                } else {
                    historical_samples(self.organisation_loss_sum).encode_reserved(output)?;
                }
                $(self.$after.encode_reserved(output)?;)*
                Ok(())
            }
            fn decode(input: &mut Decoder<'_>) -> io::Result<Self> {
                let (committed, marked) = decode_committed(input)?;
                $(let $before = <$before_ty as Value>::decode(input)?;)*
                let organisation_loss_sum = if marked {
                    <i64 as Value>::decode(input)?
                } else {
                    LandCombatSideDetails::loss_sum(&<[i64; 24] as Value>::decode(input)?)
                };
                $(let $after = <$after_ty as Value>::decode(input)?;)*
                Ok(Self {
                    committed,
                    $($before,)*
                    organisation_loss_sum,
                    $($after,)*
                })
            }
            fn visit_field_gaps(&self, visit: &mut dyn FnMut(&'static str)) {
                $(Value::visit_field_gaps(&self.$before, visit);)*
                $(Value::visit_field_gaps(&self.$after, visit);)*
            }
        }
    };
}

/// The organisation loss's width: the sum's, or the 24 samples' where the
/// test-only historical writer stands in for format 101.
fn loss_bytes() -> usize {
    if historical_stamp().is_none() {
        size_of::<i64>()
    } else {
        24 * size_of::<i64>()
    }
}

/// Samples whose sum is `sum`, for the test-only historical writer standing
/// in for format 101: the sum in the first slot. Readers take only the sum.
fn historical_samples(sum: i64) -> [i64; 24] {
    let mut samples = [0; 24];
    samples[0] = sum;
    samples
}

side_value!(
    [
        reserves: Vec<u64>,
        tactic_id: i32,
        tactic_kind: u8,
        tactic_counter_id: u32,
        tactic_token: Vec<u8>,
        air_counts: Vec<i32>,
        anti_air_attack: RetiredKept<i64>,
        air_kills: u32,
        damage: [i64; 2],
        ground_damage: RetiredNested<[i64; 2]>,
        prevented_damage: RetiredKept<[i64; 2]>,
    ],
    [
        organisation_loss_count: u32,
        commander: u64,
        commander_skill: i32,
        commander_category: i32,
        commander_country: u32,
        commander_experience: RetiredNested<i64>,
        commander_name_token: Vec<u8>,
        commander_description_token: RetiredNested<Vec<u8>>,
        commander_formation: u64,
        presentation: CombatCommanderPresentation,
        traits: Vec<LandCombatCommanderTrait>,
        timed_traits: RetiredNested<Vec<LandCombatCommanderTimedTrait>>,
        learning_traits: RetiredNested<Vec<LandCombatCommanderLearningTrait>>,
    ]
);

/// A timed trait of the side's commander.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatCommanderTimedTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    /// Signed; -1 is not an expiring trait, zero still is.
    pub remaining_days: i32,
}
value_struct!(LandCombatCommanderTimedTrait {
    token_id: i32,
    definition_ordinal: i32,
    remaining_days: i32
});

/// A learning trait the side's commander portrait lists: an entry of the commander's
/// trait-experience vector (in native order, up to its count) whose progress is strictly greater
/// than the live `LEADER_TRAITS_XP_SHOW`, decided at capture, as the portrait decides it, from the
/// exact progress. Entries at or below it are not shown and not recorded.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatLearningProgress {
    /// The trait definition's token id and ordinal, and its raw token, which the portrait localises
    /// for the commander's country.
    pub token_id: i32,
    pub definition_ordinal: i32,
    /// Fixed5 ratio of the entry's raw experience to the definition's requirement, scaling both
    /// operands before division, or `0xFFFFFFFF` when the requirement is zero or the experience a
    /// sentinel word, as the portrait computes it; stored at its displayed precision
    /// ([`LandCombatSideInputs::PROGRESS_STEP`]).
    pub progress: i64,
    pub token: Vec<u8>,
}
value_struct!(LandCombatLearningProgress {
    token_id: i32,
    definition_ordinal: i32,
    progress @ LandCombatSideInputs::PROGRESS_STEP: i64,
    token: Vec<u8>
});

/// A land combat side's hover inputs beside [`LandCombatSideDetails`]: the CAS
/// hover's anti-air and prevented damage and the selected commander's skill
/// progress, offence and defence, delayed description, timed traits and
/// shown learning traits. They are read in the same pass as the side's other
/// operands; a side without a selected commander keeps the commander fields
/// at their defaults ([`Self::NO_PROGRESS`] for the progress). The skill
/// hover's next level, its leader block and the live learning-trait floor are
/// optional reads: one that fails leaves only its part unknown
/// ([`Self::UNREAD_PROGRESS`], `None`) and the row stands.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatSideInputs {
    /// The side's anti-air cache as stored: combat-statistics refresh writes it and CAS application
    /// reads it. The opposing side's CAS hover derives anti-air effectiveness from it, whose 1.5
    /// power reads every digit, so it stays exact.
    pub anti_air_attack: i64,
    /// The CAS damage the side's anti-air prevented, strength then organisation: cumulative, added
    /// by the CAS application after its anti-air reduction. Only the opposing side's CAS hover
    /// reads them, as STR_PREVENT and ORG_PREVENT: [`Self::PREVENTED_STEP`].
    pub prevented_damage: [i64; 2],
    /// The selected commander's skill progress: experience over the next skill level's requirement
    /// (lookup at level + 1), `0xFFFFFFFF` for a zero requirement or a sentinel experience word, or
    /// [`Self::NO_PROGRESS`] when the next level is missing or invalid (the hover's skill-cap
    /// message), or [`Self::UNREAD_PROGRESS`] when the next level could not be read. The hover
    /// prints a whole percentage in a fixed colour: [`Self::PROGRESS_STEP`].
    pub skill_progress: i64,
    /// Raw keys 0x5F and 0x60 of the leader modifier block that the leader selects for
    /// the unit picked from the side's units. The commander hover copies that block into
    /// a fresh accumulator and describes it after the skill line; zero when the block
    /// lacks the key. `None`: the block, or the unit choice that selects it, could not be
    /// read.
    pub skill_modifiers: Option<[i64; 2]>,
    /// The commander's raw description token, whose localised cache ( filled during loading) is the
    /// portrait's delayed text.
    pub description_token: Vec<u8>,
    /// The commander's timed traits in native order; each trait hover's expiry line reads the first
    /// matching entry.
    pub timed_traits: Vec<LandCombatCommanderTimedTrait>,
    /// `None`: the live `LEADER_TRAITS_XP_SHOW` could not be read, so which
    /// rows the portrait shows is unknown.
    pub learning_traits: Option<Vec<LandCombatLearningProgress>>,
}
impl LandCombatSideInputs {
    /// The prevented strength and organisation damage at the precision the CAS hover
    /// prints: whole units in a fixed colour truncated by the fixed5 formatter, in every
    /// shipped language and every installed mod.
    pub const PREVENTED_STEP: crate::display::Step =
        crate::display::Step::fixed5(100_000, crate::display::Rounding::TowardZero);
    /// Progress ratios at the precision the game prints them (`%0`, whole
    /// percent: 1000 raw, truncated). The skill and learning-trait lines print
    /// nothing finer, the learning-trait threshold is decided before the step,
    /// and the step keeps the `0xFFFFFFFF` word's printed percent.
    pub const PROGRESS_STEP: crate::display::Step =
        crate::display::Step::fixed5(1_000, crate::display::Rounding::TowardZero);
    /// No skill progress: no next level, an invalid one, or no commander.
    pub const NO_PROGRESS: i64 = i64::MIN;
    /// The next level could not be read. No ratio reaches it: its divisor
    /// is a nonzero multiple of 100000, and the exceptional word is
    /// `0xFFFFFFFF`. Display steps keep both sentinel words exact.
    pub const UNREAD_PROGRESS: i64 = i64::MAX;
}
impl Default for LandCombatSideInputs {
    fn default() -> Self {
        Self {
            anti_air_attack: 0,
            prevented_damage: [0; 2],
            skill_progress: Self::NO_PROGRESS,
            skill_modifiers: Some([0; 2]),
            description_token: Vec::new(),
            timed_traits: Vec::new(),
            learning_traits: Some(Vec::new()),
        }
    }
}
value_struct!(LandCombatSideInputs {
    anti_air_attack: i64,
    prevented_damage @ LandCombatSideInputs::PREVENTED_STEP: [i64; 2],
    skill_progress @ LandCombatSideInputs::PROGRESS_STEP: i64,
    skill_modifiers: Option<[i64; 2]>,
    description_token: Vec<u8>,
    timed_traits: Vec<LandCombatCommanderTimedTrait>,
    learning_traits: Option<Vec<LandCombatLearningProgress>>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatTacticCountryWeight {
    pub country: u32,
    pub tactic_id: i32,
    pub modifier_key: u32,
    pub metadata_flags: u32,
    pub metadata_gate: i32,
    pub country_factor_raw: i64,
    pub accumulator_raw: i64,
    pub accumulator_entry_present: u8,
}
value_struct!(LandCombatTacticCountryWeight {
    country: u32,
    tactic_id: i32,
    modifier_key: u32,
    metadata_flags: u32,
    metadata_gate: i32,
    country_factor_raw: i64,
    accumulator_raw: i64,
    accumulator_entry_present: u8
});

/// The per-side simulation operands of preferred-tactic weights;
/// the candidates, weights and unlocks come from content and
/// CountryTacticUnlocks. A side whose country set emptied has no country
/// preference or weight.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatTacticInputs {
    /// Country, general and field-marshal preferred tactic ids per side.
    pub preferred_tactic_ids: [[i32; 3]; 2],
    pub country_weights: [LandCombatTacticCountryWeight; 2],
    pub country_weight_present: [u8; 2],
}
value_struct!(LandCombatTacticInputs {
    preferred_tactic_ids: [[i32; 3]; 2],
    country_weights: [LandCombatTacticCountryWeight; 2],
    country_weight_present: [u8; 2]
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandDivisionModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(LandDivisionModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandDivisionWidthTerm {
    pub subunit: u32,
    pub count: i32,
    pub width: i64,
}
value_struct!(LandDivisionWidthTerm {
    subunit: u32,
    count: i32,
    width: i64
});

/// Exile-frame operands for a committed division whose template is exiled:
/// origin type exactly 2.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatExileInputs {
    pub origin_type: i32,
    /// The positive template origin country, otherwise its fallback country.
    pub country: i32,
    pub country_fallback: i32,
    /// The location state's core countries and the exile country's occupier
    /// set, raw tags in native order.
    pub location_cores: Vec<u32>,
    pub occupiers: Vec<u32>,
}
value_struct!(LandCombatExileInputs { origin_type: i32, country: i32, country_fallback: i32, location_cores: Vec<u32>, occupiers: Vec<u32> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandDivisionContext {
    pub template: u64,
    pub leader_modifier_kind: u8,
    pub country_modifiers: [LandDivisionModifier; 3],
    pub leader_modifiers: [LandDivisionModifier; 3],
    pub modifier_metadata: [[u32; 2]; 3],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(LandDivisionContext {
    template: u64,
    leader_modifier_kind: u8,
    country_modifiers: [LandDivisionModifier; 3],
    leader_modifiers: [LandDivisionModifier; 3],
    modifier_metadata: [[u32; 2]; 3],
    feature_flags: u32,
    eligibility_callback: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatSummaryModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(NavalCombatSummaryModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatSummaryInputs {
    pub recent_join_penalty: i64,
    pub escort_bonus: i64,
    /// The side's ordered primary country list, raw references.
    pub countries: Vec<u32>,
    /// Its ordered roster-owner list, which admits a viewer to the combat's
    /// shared vision.
    pub secondary_countries: Vec<u32>,
    pub modifiers: [[NavalCombatSummaryModifier; 5]; 5],
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(NavalCombatSummaryInputs { recent_join_penalty: i64, escort_bonus: i64, countries: Vec<u32>, secondary_countries: Vec<u32>, modifiers: [[NavalCombatSummaryModifier; 5]; 5], modifier_metadata: [[u32; 2]; 5], feature_flags: u32, eligibility_callback: u8 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatAirEfficiencyModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(NavalCombatAirEfficiencyModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatAirEfficiency {
    pub source: u8,
    pub pool_type_flags: u64,
    pub mission: i32,
    pub container_present: u8,
    pub base_crowding: i64,
    pub night: i64,
    pub modifiers: [NavalCombatAirEfficiencyModifier; 3],
    /// Definition-table flags and feature selector of keys 0x22, 0x23 and
    /// the static screening sortie keys 0x24, 0x1B6, read whatever holds them.
    pub modifier_metadata: [[u32; 2]; 4],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(NavalCombatAirEfficiency {
    source: u8,
    pool_type_flags: u64,
    mission: i32,
    container_present: u8,
    base_crowding: i64,
    night: i64,
    modifiers: [NavalCombatAirEfficiencyModifier; 3],
    modifier_metadata: [[u32; 2]; 4],
    feature_flags: u32,
    eligibility_callback: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatAirWingKill {
    pub wing: u64,
    pub index: u32,
    pub category: u64,
    pub count: i32,
}
value_struct!(NavalCombatAirWingKill {
    wing: u64,
    index: u32,
    category: u64,
    count: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatPlane {
    /// Sub-unit definition indices for wing references, aligned by occurrence.
    /// MAX means the historical wing no longer resolves. They supply the
    /// hover's localised short-name list.
    pub wing_types: Vec<u32>,
    pub source: u8,
    /// Side forward-group ordinal, or MAX for a paired-only group.
    pub group_index: u32,
    pub index: u32,
    pub carrier_slot: i32,
    /// Whether the shared air object's carrier-member pointer is present.
    pub carrier_member_present: u8,
    pub carrier_state: u32,
    pub air_base: u64,
    pub wings: [Vec<u64>; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub casualties: i32,
    pub time_duration: i32,
    pub country: u32,
    pub date: i32,
    pub flags: [u8; 2],
    pub variant: u64,
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
    pub kills: Vec<NavalCombatAirWingKill>,
    pub efficiency: NavalCombatAirEfficiency,
}
value_struct!(NavalCombatPlane { wing_types: Vec<u32>, source: u8, group_index: u32, index: u32, carrier_slot: i32, carrier_member_present: u8, carrier_state: u32, air_base: u64, wings: [Vec<u64>;2], initial_planes: i32, current_planes: i32, casualties: i32, time_duration: i32, country: u32, date: i32, flags: [u8; 2], variant: u64, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2], kills: Vec<NavalCombatAirWingKill>, efficiency: NavalCombatAirEfficiency });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatSideDetails {
    pub damage_history: [[i64; 6]; 6],
    pub original_health: i64,
    pub total_damage: i64,
    pub damage_by_type: Vec<NavalDamageSource>,
    pub commander: u64,
    pub engaged: u8,
    pub retreating: u8,
    pub capital_screening: i64,
    pub carrier_screening: i64,
    pub positioning: i64,
    pub anti_air: i64,
    pub planes: Vec<NavalCombatPlane>,
    pub summary: NavalCombatSummaryInputs,
    pub commander_inputs: Option<NavalCommanderInputs>,
    /// The side's task-force handles in stored order. Only navies are admitted,
    /// and the save writes each as `unit`. Engaged-flag derivation compares
    /// these handles hour over hour.
    pub units: Vec<u64>,
}
value_struct!(NavalCombatSideDetails { damage_history: [[i64; 6]; 6], original_health: i64, total_damage: i64, damage_by_type: Vec<NavalDamageSource>, commander: u64, engaged: u8, retreating: u8, capital_screening: i64, carrier_screening: i64, positioning: i64, anti_air: i64, planes: Vec<NavalCombatPlane>, summary: NavalCombatSummaryInputs, commander_inputs: Option<NavalCommanderInputs>, units: Vec<u64> });

/// One source-token total in the native damage-by-source map's iteration order.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalDamageSource {
    pub token: u32,
    pub damage: i64,
}
value_struct!(NavalDamageSource {
    token: u32,
    damage: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatDamage {
    pub source: u64,
    pub country: u32,
    pub damage: i64,
}
value_struct!(NavalCombatDamage {
    source: u64,
    country: u32,
    damage: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatAirHit {
    pub variant: u64,
    pub count: i32,
    pub country: u32,
}
value_struct!(NavalCombatAirHit {
    variant: u64,
    count: i32,
    country: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatHit {
    pub target_slot: u32,
    pub target_name: Vec<u8>,
    pub convoy: u8,
    pub damage: i64,
    pub denominator: i64,
    pub last_hit: u8,
}
value_struct!(NavalCombatHit { target_slot: u32, target_name: Vec<u8>, convoy: u8, damage: i64, denominator: i64, last_hit: u8 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatLastTarget {
    pub present: u8,
    pub is_convoy: u8,
    pub target_group_member_count: i32,
}
value_struct!(NavalCombatLastTarget {
    present: u8,
    is_convoy: u8,
    target_group_member_count: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalShipModifierTerm {
    pub key: u32,
    pub raw: i64,
    pub metadata_flags: u32,
    pub feature_selector: u32,
}
value_struct!(NavalShipModifierTerm {
    key: u32,
    raw: i64,
    metadata_flags: u32,
    feature_selector: u32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalShipModifierSource {
    pub scope: u8,
    pub index: u32,
    pub selector: u32,
    pub reference: u64,
    pub weights: [i64; 2],
    pub movement: i64,
    pub dynamic_keys: [u32; 2],
    pub terms: Vec<NavalShipModifierTerm>,
}
value_struct!(NavalShipModifierSource { scope: u8, index: u32, selector: u32, reference: u64, weights: [i64; 2], movement: i64, dynamic_keys: [u32; 2], terms: Vec<NavalShipModifierTerm> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalShipWeaponContext {
    pub task_force: u64,
    pub country: u32,
    pub commander: u64,
    pub ship_class: u32,
    pub subunit: u32,
    pub weapon_bases: [i64; 5],
    pub weather_factor: i64,
    pub fuel: [i64; 2],
    pub night: i64,
    pub formation_flags: u64,
    pub detection_base: i64,
    pub air_base: u64,
    pub air_base_maximum: i32,
    pub air_detection_base: i64,
    pub terrain_name: Vec<u8>,
    pub sources: Vec<NavalShipModifierSource>,
}
value_struct!(NavalShipWeaponContext { task_force: u64, country: u32, commander: u64, ship_class: u32, subunit: u32, weapon_bases: [i64; 5], weather_factor: i64, fuel: [i64; 2], night: i64, formation_flags: u64, detection_base: i64, air_base: u64, air_base_maximum: i32, air_detection_base: i64, terrain_name: Vec<u8>, sources: Vec<NavalShipModifierSource> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalResultShip {
    pub member: u64,
    pub owner: u32,
    pub subunit_index: i32,
    pub formation_type: i32,
    pub build_cost_ic: i64,
    pub strength_ratio: i64,
    pub pride: u8,
    pub is_convoy: u8,
    pub name: Vec<u8>,
    pub type_name: Vec<u8>,
    pub sprite: Vec<u8>,
    pub sunk_by: Vec<u8>,
    pub variant: u64,
    pub convoy_client: u64,
    pub convoy_index: i32,
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
}
value_struct!(NavalResultShip { member: u64, owner: u32, subunit_index: i32, formation_type: i32, build_cost_ic: i64, strength_ratio: i64, pride: u8, is_convoy: u8, name: Vec<u8>, type_name: Vec<u8>, sprite: Vec<u8>, sunk_by: Vec<u8>, variant: u64, convoy_client: u64, convoy_index: i32, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalResultPlane {
    pub variant: u64,
    pub variant_summary_inputs: [i64; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub lost_planes: i32,
    pub country: u32,
    pub name: Vec<u8>,
    pub naval_hits: Vec<NavalCombatHit>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
}
value_struct!(NavalResultPlane { variant: u64, variant_summary_inputs: [i64; 2], initial_planes: i32, current_planes: i32, lost_planes: i32, country: u32, name: Vec<u8>, naval_hits: Vec<NavalCombatHit>, air_hits: [Vec<NavalCombatAirHit>;2] });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalResultSideDetails {
    pub leader: u64,
    pub hq_leader: u64,
    pub hq_influences: Vec<u64>,
    pub ships: Vec<NavalResultShip>,
    pub planes: Vec<NavalResultPlane>,
    pub hq_modifiers: Vec<NavalShipModifierSource>,
    pub commander_context: Option<NavalCommanderInputs>,
    pub hq_commander_context: Option<NavalCommanderInputs>,
}
value_struct!(NavalResultSideDetails { leader: u64, hq_leader: u64, hq_influences: Vec<u64>, ships: Vec<NavalResultShip>, planes: Vec<NavalResultPlane>, hq_modifiers: Vec<NavalShipModifierSource>, commander_context: Option<NavalCommanderInputs>, hq_commander_context: Option<NavalCommanderInputs> });

/// Stored border-war simulation operands, sampled only for border-war combats.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandBorderWarSide {
    pub country: i32,
    pub state: i32,
    /// Exact MSVC identifier bytes of the win and lose events. Their lengths are the native
    /// presence tests, not event pointers.
    pub win_event: Vec<u8>,
    pub lose_event: Vec<u8>,
}
value_struct!(LandBorderWarSide { country: i32, state: i32, win_event: Vec<u8>, lose_event: Vec<u8> });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandBorderWarInputs {
    /// Exact transfer-state byte; any nonzero transfers the state.
    pub transfer_state: u8,
    pub sides: [LandBorderWarSide; 2],
    /// Stored fixed5 base width.
    pub combat_width: Recorded<i64>,
}
value_struct!(LandBorderWarInputs {
    transfer_state: u8,
    sides: [LandBorderWarSide; 2]
} appended LandBorderWarInputs {
    combat_width: Recorded<i64>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatDetailsRow {
    pub combat: u64,
    pub province: u32,
    pub phase: i32,
    pub width_factor: i64,
    pub attacker_bonus: i64,
    pub defender_bonus: i64,
    pub stored_terrain_valid: u8,
    pub stored_terrain_name: Vec<u8>,
    pub sides: [LandCombatSideDetails; 2],
    pub tactic_inputs: LandCombatTacticInputs,
    pub border_war: Recorded<Option<LandBorderWarInputs>>,
    /// Attacker then defender, as `sides`. Its own field, so a side whose
    /// window operands change every hour (the loss ring, membership) leaves
    /// these unchanged; they change when the CAS counters, the commander or
    /// a displayed percent do. NotRecorded before
    /// [`crate::rows::compatibility::SIDE_INPUTS_FORMAT`].
    pub side_inputs: Recorded<[LandCombatSideInputs; 2]>,
}
typed_codec!(LandCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (province): U32, 1 => (phase): I32, 2 => (width_factor): I64, 3 => (attacker_bonus): I64, 4 => (defender_bonus): I64, 5 => (stored_terrain_valid): U8, 6 => (stored_terrain_name): Bytes, 7 => (sides): Opaque, 8 => (tactic_inputs): Opaque, 9 => (border_war): Opaque, 10 => (side_inputs): Opaque }, |row| { let _ = row; Ok(()) });

impl LandCombatDetailsRow {
    /// A side's CAS hover operands: its anti-air cache and its prevented strength and
    /// organisation damage, from `side_inputs`, or from the side's own parts in a
    /// recording older than [`crate::rows::compatibility::SIDE_INPUTS_FORMAT`], which
    /// holds them there exact. None when the recording holds neither.
    pub fn cas_inputs(&self, side: usize) -> Option<(i64, [i64; 2])> {
        match &self.side_inputs {
            Recorded::Value(inputs) => {
                Some((inputs[side].anti_air_attack, inputs[side].prevented_damage))
            }
            Recorded::NotRecorded => {
                let side = &self.sides[side];
                Some((
                    *side.anti_air_attack.older()?,
                    *side.prevented_damage.older()?,
                ))
            }
        }
    }
}

/// A land combat member's division values: what its DivisionStatus, DivisionPanelStats and
/// DivisionMovement rows of the same hour record for the division, read from the same native
/// words at the same moment.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatMemberValues {
    /// DivisionStatus' naming operands. The viewer renders the name from content.
    pub naming: NameOperands,
    /// DivisionStatus' strength factor.
    pub strength_combat_factor: i64,
    /// DivisionMovement's fuel, and the stat block's fuel capacity (DivisionStatus' movement
    /// fuel rate).
    pub fuel_inputs: [i64; 2],
    /// Defence, breakthrough, hardness, soft and hard attack: the panel's selectors 1 to 5.
    pub stat_values: [i64; 5],
    /// The panel's selectors 0x3f and 0x40.
    pub armour: i64,
    pub piercing: i64,
    /// The stat block's anti-air word (DivisionStatus' movement air defense).
    pub air_attack: i64,
    /// The panel's selector 6.
    pub reconnaissance: i64,
    /// The stat block's initiative (the panel's selector 8), which reinforcement chance scales by
    /// the country's and the contextual leader's 0x250 keys and `ARMY_INITIATIVE_REINFORCE_FACTOR`.
    /// Exact: those products truncate at every step.
    pub initiative: i64,
    /// The stat block's maximum speed (the panel's selector 0x3e).
    pub base_speed: i64,
}
value_struct!(LandCombatMemberValues {
    naming: NameOperands,
    strength_combat_factor: i64,
    fuel_inputs: [i64; 2],
    stat_values: [i64; 5],
    armour: i64,
    piercing: i64,
    air_attack: i64,
    reconnaissance: i64,
    initiative: i64,
    base_speed: i64
});
opaque_value!(Option<LandCombatMemberValues>);

impl LandCombatMemberValues {
    /// Where [`DivisionPanelStatsRow::panel_stats`] holds selectors 1 to 5, 0x3f, 0x40, 6 and 8.
    pub const STATS: [usize; 5] = [11, 12, 20, 9, 10];
    pub const ARMOUR: usize = 13;
    pub const PIERCING: usize = 14;
    pub const RECONNAISSANCE: usize = 3;
    pub const INITIATIVE: usize = 15;

    /// The division's values from its rows of the hour; None when one of them lacks a value.
    pub fn joined(
        panel: &DivisionPanelStatsRow,
        status: &DivisionStatusRow,
        movement: &DivisionMovementRow,
    ) -> Option<Self> {
        Self::joined_of(
            &|at| panel.panel_stats[at],
            &status.naming,
            &status.movement_stats,
            status.strength_factor,
            movement.fuel,
        )
    }

    /// [`Self::joined`] from values: the panel's words by index, the status' naming, movement
    /// stats and strength factor, and the movement's fuel.
    pub fn joined_of(
        word: &dyn Fn(usize) -> Option<i64>,
        naming: &Recorded<Option<NameOperands>>,
        movement_stats: &Recorded<Option<DivisionMovementStats>>,
        strength_factor: i64,
        fuel: Recorded<Option<i64>>,
    ) -> Option<Self> {
        let Recorded::Value(Some(naming)) = naming else {
            return None;
        };
        let Recorded::Value(Some(stats)) = movement_stats else {
            return None;
        };
        let Recorded::Value(Some(fuel)) = fuel else {
            return None;
        };
        let mut stat_values = [0; 5];
        for (value, at) in stat_values.iter_mut().zip(Self::STATS) {
            *value = word(at)?;
        }
        Some(Self {
            naming: naming.clone(),
            strength_combat_factor: strength_factor,
            fuel_inputs: [fuel, stats.fuel_rate],
            stat_values,
            armour: word(Self::ARMOUR)?,
            piercing: word(Self::PIERCING)?,
            air_attack: stats.air_defense,
            reconnaissance: word(Self::RECONNAISSANCE)?,
            initiative: word(Self::INITIATIVE)?,
            base_speed: word(DivisionPanelStatsRow::MAXIMUM_SPEED)?,
        })
    }
}

/// LandCombatDivisions: one combat member, keyed (combat, side, ordinal in the side's native
/// participant walk). Its division's values are the division's rows of the same hour
/// ([`Self::values`]); a member stores its own copy only where those rows do not hold them, as
/// for a division no roster lists.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatDivisionRow {
    pub combat: u64,
    pub side: u8,
    pub ordinal: u32,
    pub role: u8,
    pub province: u32,
    pub division: u64,
    /// The division's commander handle; its name, skill and traits are the
    /// UnitLeaders and UnitLeaderTraits rows of the same hour.
    pub commander: u64,
    /// The attack and defence accumulators and the invasion attack and defence scales.
    pub scales: [i64; 4],
    pub context: LandDivisionContext,
    pub width_terms: Vec<LandDivisionWidthTerm>,
    /// Raw unit latch word of the direction count.
    pub direction_group: i32,
    pub exile: Option<LandCombatExileInputs>,
    /// The unit stat block's stored equipment category mask. A category trigger
    /// counts land combatant units whose masks meet its category mask.
    pub categories: u64,
    /// A reserve's reinforcement speed share: its base speed (the stat block's maximum speed) times
    /// the movement-speed modifier (location, next route province, ignoring terrain and combat),
    /// over the live `NMilitary.SLOWEST_SPEED`, less one, clamped to [0, 2] (fixed5). The
    /// reinforcement chance's speed arm multiplies it by `SPEED_REINFORCEMENT_BONUS`. As
    /// DivisionStatus keeps the camouflage category decided with the live define, the collector
    /// composes the share at capture (`engine_native::land_reinforcement_inputs::speed`) and records it
    /// instead of the drifting movement-speed operands it takes. `None` for a committed division,
    /// and for a reserve whose operands could not be read or take a form the port does not follow
    /// (counted in collector status). Exact: the arm's truncating product and the chance's sum read
    /// every digit.
    pub reinforcement_speed: Option<i64>,
    /// The member's own copy of its division's values, where its division's rows of the hour do
    /// not hold the values it read ([`Self::values`]).
    pub own: Option<LandCombatMemberValues>,
}
impl LandCombatDivisionRow {
    /// The member's division values: its own copy, or else its division's rows of the same hour.
    pub fn values(
        &self,
        panel: Option<&DivisionPanelStatsRow>,
        status: Option<&DivisionStatusRow>,
        movement: Option<&DivisionMovementRow>,
    ) -> Option<LandCombatMemberValues> {
        Self::values_of(self.own.as_ref(), || LandCombatMemberValues::joined(panel?, status?, movement?))
    }

    /// [`Self::values`] from values: the member's own copy, else its division's rows joined.
    pub fn values_of(
        own: Option<&LandCombatMemberValues>,
        joined: impl FnOnce() -> Option<LandCombatMemberValues>,
    ) -> Option<LandCombatMemberValues> {
        if let Some(own) = own {
            return Some(own.clone());
        }
        joined()
    }
    /// The reinforcement calculation clamps the reserve speed share to [0, 0x30D40].
    pub const SPEED_SHARE_MAXIMUM: i64 = 200_000;
}

/// A land combat member row with its division values ([`LandCombatDivisionRow::values`]); it
/// reads as its row.
#[derive(Debug, Clone, Copy)]
pub struct LandCombatMember<'a> {
    pub row: &'a LandCombatDivisionRow,
    pub values: Option<&'a LandCombatMemberValues>,
}
impl std::ops::Deref for LandCombatMember<'_> {
    type Target = LandCombatDivisionRow;
    fn deref(&self) -> &LandCombatDivisionRow {
        self.row
    }
}

/// Each member's division values, in row order: its own copy, or its division's rows among the
/// hour's DivisionPanelStats, DivisionStatus and DivisionMovement rows, each in division order.
pub fn land_combat_member_values(
    members: &[LandCombatDivisionRow],
    panels: &[DivisionPanelStatsRow],
    statuses: &[DivisionStatusRow],
    movements: &[DivisionMovementRow],
) -> Vec<Option<LandCombatMemberValues>> {
    fn find<T>(rows: &[T], division: u64, key: impl Fn(&T) -> u64) -> Option<&T> {
        rows.binary_search_by_key(&division, key)
            .ok()
            .map(|at| &rows[at])
    }
    members
        .iter()
        .map(|member| {
            member.values(
                find(panels, member.division, |row| row.division),
                find(statuses, member.division, |row| row.division),
                find(movements, member.division, |row| row.division),
            )
        })
        .collect()
}

/// The members, each with its values from [`land_combat_member_values`].
pub fn land_combat_members<'a>(
    members: &'a [LandCombatDivisionRow],
    values: &'a [Option<LandCombatMemberValues>],
) -> impl Iterator<Item = LandCombatMember<'a>> + Clone {
    members
        .iter()
        .zip(values)
        .map(|(row, values)| LandCombatMember {
            row,
            values: values.as_ref(),
        })
}
typed_codec!(LandCombatDivisionRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.ordinal as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (role): U8, 1 => (province): U32, 2 => (division): U64, 3 => (commander): U64, 4 => (scales[0]): I64, 5 => (scales[1]): I64, 6 => (scales[2]): I64, 7 => (scales[3]): I64, 8 => (context): Opaque, 9 => (width_terms): Opaque, 10 => (direction_group): I32, 11 => (exile): Opaque, 12 => (categories): U64, 13 => (reinforcement_speed): OptionalI64, 14 => (own): Opaque }, |row| { if row.side > 1 { return Err(invalid("invalid combat side")); }
    if matches!(row.reinforcement_speed, Some(share) if !(0..=LandCombatDivisionRow::SPEED_SHARE_MAXIMUM).contains(&share)) { return Err(invalid("reserve speed share outside reinforcement bounds")); } Ok(()) });

/// LandCombatModifiers: one row a side of each combat whose modifier context
/// (`cland_combat::read_modifiers_context`) was read, published with that
/// read alone. Its members' entries are on LandCombatDivisionModifiers rows.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatModifierRow {
    pub combat: u64,
    pub side: u8,
    /// Modifier, dig-in factor and terrain factor; present only in a A border war combat,
    /// whose window title, combatant class and global modifier hover read them.
    pub border_values: Option<[i64; 3]>,
    /// The side's members' entries, a list a side before
    /// [`crate::rows::compatibility::LAND_MODIFIER_ROWS_FORMAT`], which
    /// LandCombatDivisionModifiers holds a row a member since.
    pub divisions: Retired,
}
typed_codec!(LandCombatModifierRow, 65, |row| (row.combat as u128) << 1 | row.side as u128,
 |key| Self { combat: (key >> 1) as u64, side: (key & 1) as u8, ..Default::default() },
 { 0 => (border_values): Opaque, 1 => (divisions): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

/// LandCombatDivisionModifiers: one combat member's ordered combat modifier
/// entries (id, A, B), keyed as its LandCombatDivisions row (combat, side,
/// ordinal in the side's native participant walk), which names the division
/// ([`Self::division_in`]). Every participant the walk visits has a row, an empty
/// entry list included. One row a member, so an hour that changes one
/// division's entries stores only that row.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatDivisionModifierRow {
    pub combat: u64,
    pub side: u8,
    pub ordinal: u32,
    pub entries: Vec<CombatModifierValue>,
    /// The member's division, where no LandCombatDivisions row of the hour names it.
    pub division: Option<u64>,
}
impl LandCombatDivisionModifierRow {
    /// The member's division: its own, or else its member row's among `members` (in key order).
    pub fn division_in(&self, members: &[LandCombatDivisionRow]) -> Option<u64> {
        self.division.or_else(|| {
            members
                .binary_search_by_key(&self.key(), RowCodec::key)
                .ok()
                .map(|at| members[at].division)
        })
    }
}
typed_codec!(LandCombatDivisionModifierRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.ordinal as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, ordinal: key as u32, ..Default::default() },
 { 0 => (entries): Opaque, 1 => (division): OptionalU64 }, |row| { if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

/// Hourly side operands of one land combat, for its map marker and window. The country
/// sets keep raw references in native order; the viewer applies the visibility and side
/// rules for any viewing country.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct LandCombatSideRow {
    pub combat: u64,
    /// The marker's frame and the window's indicator icon.
    pub indicator: u8,
    /// Attacker then defender land-combatant flag words.
    pub flags: [u32; 2],
    pub attacker_primary: Vec<u32>,
    pub attacker_secondary: Vec<u32>,
    pub defender_primary: Vec<u32>,
    pub defender_secondary: Vec<u32>,
}
typed_codec!(LandCombatSideRow, 64, |row| row.combat as u128,
|key| Self { combat: key as u64, ..Default::default() },
{ 0 => (indicator): U8, 1 => (flags): Opaque, 2 => (attacker_primary): U32List, 3 => (attacker_secondary): U32List, 4 => (defender_primary): U32List, 5 => (defender_secondary): U32List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatDetailsRow {
    pub combat: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    pub progress: i64,
    pub sides: [NavalCombatSideDetails; 2],
    /// Active i16 weather values, little endian, in this live combat's
    /// strategic template order.
    pub custom_sea: Option<Vec<u8>>,
    /// 0 complete; 1 guarded custom-grid read failed. No diagnostic text is
    /// serialized per combat.
    pub custom_sea_error: u8,
}
typed_codec!(NavalCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (province): U32, 1 => (day): I32, 2 => (duration): I32, 3 => (progress): I64, 4 => (sides): Opaque, 5 => (custom_sea): Opaque, 6 => (custom_sea_error): U8 }, |row| { if row.custom_sea.as_ref().is_some_and(|sea| sea.len() % 2 != 0) { return Err(invalid("odd custom sea i16 bytes")); } else if row.custom_sea_error > 1 || (row.custom_sea.is_some() == (row.custom_sea_error != 0)) { return Err(invalid("custom sea capture status conflicts with values")); } Ok(()) });

/// Ordered wing-handle occurrences in the dense wing walk for a base
/// referenced by naval combat. Repeated handles are meaningful. Every
/// resolved referenced base has a row, including one with an empty wing list.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatAirBaseWingHandlesRow {
    pub base: u64,
    pub wings: Vec<u64>,
}
typed_codec!(NavalCombatAirBaseWingHandlesRow, 64, |row| row.base as u128,
 |key| Self { base: key as u64, ..Default::default() },
 { 0 => (wings): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatShipRow {
    pub combat: u64,
    pub side: u8,
    pub battle_slot: u32,
    pub ship: u64,
    pub owner: u32,
    pub state: u32,
    pub member_source: u8,
    pub group_index: u32,
    pub member_index: u32,
    pub convoy: u64,
    pub subunit_index: i32,
    pub formation_type: i32,
    pub build_cost_ic: i64,
    pub cached_strength_ratio: i64,
    pub pride: u8,
    pub is_convoy: u8,
    pub name: Vec<u8>,
    pub type_name: Vec<u8>,
    pub sprite: Vec<u8>,
    pub sunk_by: Vec<u8>,
    pub variant: u64,
    pub arrival_hours: i32,
    pub received_criticals: i32,
    pub evacuation_state: i32,
    pub detection_state: i32,
    pub escape_progress: i64,
    pub escape_speed_base: i64,
    pub last_target: NavalCombatLastTarget,
    pub received_damage: [i64; 6],
    pub live_ship: u8,
    pub live_convoy: u8,
    pub strength: i64,
    pub organisation: i64,
    pub base_organisation: i64,
    pub maximum_hit_points: i64,
    pub formation_flags: u64,
    pub weapon_context: NavalShipWeaponContext,
    pub damage_entries: Vec<NavalCombatDamage>,
    pub air_hits: [Vec<NavalCombatAirHit>; 2],
    pub naval_hits: Vec<NavalCombatHit>,
}
// Display steps ([`crate::display`]). Strength and its maximum (the window
// groups damaged ships by `strength < maximum_hit_points`), the received damage
// lanes, the weapon context (night's `> 50000`, the fuel ratio) and the
// oracle-only caches stay exact.
typed_codec!(NavalCombatShipRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32, ..Default::default() },
 { 0 => (ship): U64, 1 => (owner): U32, 2 => (state): U32, 3 => (member_source): U8, 4 => (group_index): U32, 5 => (member_index): U32, 6 => (convoy): U64, 7 => (subunit_index): I32, 8 => (formation_type): I32, 9 => (build_cost_ic): I64, 10 => (cached_strength_ratio): I64, 11 => (pride): U8, 12 => (is_convoy): U8, 13 => (name): Bytes, 14 => (type_name): Bytes, 15 => (sprite): Bytes, 16 => (sunk_by): Bytes, 17 => (variant): U64, 18 => (arrival_hours): I32, 19 => (received_criticals): I32, 20 => (evacuation_state): I32, 21 => (detection_state): I32, 22 => (escape_progress): I64, 23 => (escape_speed_base): I64, 24 => (last_target): Opaque, 25 => (received_damage): Opaque, 26 => (live_ship): U8, 27 => (live_convoy): U8, 28 => (strength): I64, 29 => (organisation): I64, 30 => (base_organisation): I64, 31 => (maximum_hit_points): I64, 32 => (formation_flags): U64, 33 => (weapon_context): Opaque, 34 => (damage_entries): Opaque, 35 => (air_hits): Opaque, 36 => (naval_hits): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct NavalCombatResultDetailsRow {
    pub result: u64,
    pub sides: [NavalResultSideDetails; 2],
}
typed_codec!(NavalCombatResultDetailsRow, 64, |row| row.result as u128,
 |key| Self { result: key as u64, ..Default::default() },
 { 0 => (sides): Opaque }, |row| { let _ = row; Ok(()) });

opaque_value!(LandCombatTacticInputs);
opaque_value!(Option<LandBorderWarInputs>);
opaque_value!(LandDivisionContext);
opaque_value!(NavalCombatLastTarget);
opaque_value!(NavalShipWeaponContext);
opaque_value!(Option<[i64; 3]>);
opaque_value!(Option<LandCombatExileInputs>);
opaque_value!(Vec<CombatModifierValue>);
opaque_value!(Vec<LandDivisionWidthTerm>);
opaque_value!(Vec<NavalCombatDamage>);
opaque_value!(Vec<NavalCombatHit>);
opaque_value!([LandCombatSideDetails; 2]);
opaque_value!([LandCombatSideInputs; 2]);
opaque_value!([NavalCombatSideDetails; 2]);
opaque_value!([NavalResultSideDetails; 2]);
opaque_value!([Vec<NavalCombatAirHit>; 2]);
opaque_value!([i64; 2]);
opaque_value!([i64; 3]);
opaque_value!([i64; 4]);
opaque_value!([i64; 5]);
opaque_value!(RetiredKept<[i64; 2]>);
opaque_value!(RetiredKept<[i64; 4]>);
opaque_value!(RetiredKept<[i64; 5]>);
opaque_value!([i64; 6]);
opaque_value!([[i64; 6]; 6]);
opaque_value!([u32; 2]);
opaque_value!([u32; 4]);
