use super::*;
use replay_rows::value::value_struct;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

impl OpaqueValue for CombatModifierValue {}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatCommanderTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub kind: u32,
    pub relevant: u8,
    pub token: Vec<u8>,
}
value_struct!(LandCombatCommanderTrait { token_id: i32, definition_ordinal: i32, kind: u32, relevant: u8, token: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatCommanderLearningTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub required_experience: i32,
    pub experience: i64,
    pub token: Vec<u8>,
}
value_struct!(LandCombatCommanderLearningTrait { token_id: i32, definition_ordinal: i32, required_experience: i32, experience: i64, token: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatCommanderPortraitTooltip {
    pub source: u8,
    pub bucket: u32,
    pub key: u32,
    pub text: Vec<u8>,
}
value_struct!(LandCombatCommanderPortraitTooltip { source: u8, bucket: u32, key: u32, text: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatCommanderPortraitEntry {
    pub role: u32,
    pub size: u32,
    pub is_set: u8,
    pub kind: u32,
    pub text: Vec<u8>,
}
value_struct!(CombatCommanderPortraitEntry { role: u32, size: u32, is_set: u8, kind: u32, text: Vec<u8> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub timed_traits: Recorded<Vec<LandCombatCommanderTimedTrait>>,
}
value_struct!(NavalCommanderInputs { commander: u64, country: u32, category: i32, experience: i64, skills: [i32; 5], skill_definitions: [CombatCommanderSkillDefinition;5], next_skill: CombatCommanderSkillDefinition, name: Vec<u8>, description: Vec<u8>, presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>, learning_traits: Vec<LandCombatCommanderLearningTrait>, learning_threshold: i64, modifiers: Vec<LandCombatCommanderPortraitModifier>, modifier_feature_flags: u32, modifier_callback_kind: u8, tooltips: Vec<LandCombatCommanderPortraitTooltip> } appended NavalCommanderInputs { timed_traits: Recorded<Vec<LandCombatCommanderTimedTrait>> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatSideDetails {
    pub committed: Vec<u64>,
    pub reserves: Vec<u64>,
    pub tactic_id: i32,
    pub tactic_kind: u8,
    pub tactic_counter_id: u32,
    pub tactic_token: Vec<u8>,
    pub air_counts: Vec<i32>,
    pub anti_air_attack: RetiredKept<i64>,
    pub air_kills: u32,
    pub damage: [i64; 2],
    pub ground_damage: RetiredNested<[i64; 2]>,
    pub prevented_damage: RetiredKept<[i64; 2]>,
    pub organisation_loss_sum: i64,
    pub organisation_loss_count: u32,
    pub commander: u64,
    pub commander_skill: i32,
    pub commander_category: i32,
    pub commander_country: u32,
    pub commander_experience: RetiredNested<i64>,
    pub commander_name_token: Vec<u8>,
    pub commander_description_token: RetiredNested<Vec<u8>>,
    pub commander_formation: u64,
    pub presentation: CombatCommanderPresentation,
    pub traits: Vec<LandCombatCommanderTrait>,
    pub timed_traits: RetiredNested<Vec<LandCombatCommanderTimedTrait>>,
    pub learning_traits: RetiredNested<Vec<LandCombatCommanderLearningTrait>>,
}
impl LandCombatSideDetails {
    pub fn loss_sum(samples: &[i64; 24]) -> i64 {
        samples
            .iter()
            .fold(0_i64, |sum, value| sum.wrapping_add(*value))
    }
}

const MARKED_COMMITTED: u32 = u32::MAX / 2;

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

fn decode_committed(input: &mut Decoder<'_>) -> io::Result<(Vec<u64>, bool)> {
    if input.format < replay_rows::rows::compatibility::LOSS_SUM_FORMAT {
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
            const HAS_STEPS: bool = replay_rows::value::any(&[
                $(<$before_ty as Value>::HAS_STEPS,)*
                $(<$after_ty as Value>::HAS_STEPS,)*
            ]);

            fn round_display(&mut self) {
                $(replay_rows::value::round_field(&mut self.$before, None);)*
                $(replay_rows::value::round_field(&mut self.$after, None);)*
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

fn loss_bytes() -> usize {
    if historical_stamp().is_none() {
        size_of::<i64>()
    } else {
        24 * size_of::<i64>()
    }
}

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatCommanderTimedTrait {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub remaining_days: i32,
}
value_struct!(LandCombatCommanderTimedTrait {
    token_id: i32,
    definition_ordinal: i32,
    remaining_days: i32
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatLearningProgress {
    pub token_id: i32,
    pub definition_ordinal: i32,
    pub progress: i64,
    pub token: Vec<u8>,
}
value_struct!(LandCombatLearningProgress {
    token_id: i32,
    definition_ordinal: i32,
    progress @ LandCombatSideInputs::PROGRESS_STEP: i64,
    token: Vec<u8>
});

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatSideInputs {
    pub anti_air_attack: i64,
    pub prevented_damage: [i64; 2],
    pub skill_progress: i64,
    pub skill_modifiers: Option<[i64; 2]>,
    pub description_token: Vec<u8>,
    pub timed_traits: Vec<LandCombatCommanderTimedTrait>,
    pub learning_traits: Option<Vec<LandCombatLearningProgress>>,
}
impl LandCombatSideInputs {
    pub const PREVENTED_STEP: replay_rows::display::Step =
        replay_rows::display::Step::fixed5(100_000, replay_rows::display::Rounding::TowardZero);
    pub const PROGRESS_STEP: replay_rows::display::Step =
        replay_rows::display::Step::fixed5(1_000, replay_rows::display::Rounding::TowardZero);
    pub const NO_PROGRESS: i64 = i64::MIN;
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatTacticInputs {
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandDivisionModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(LandDivisionModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatExileInputs {
    pub origin_type: i32,
    pub country: i32,
    pub country_fallback: i32,
    pub location_cores: Vec<u32>,
    pub occupiers: Vec<u32>,
}
value_struct!(LandCombatExileInputs { origin_type: i32, country: i32, country_fallback: i32, location_cores: Vec<u32>, occupiers: Vec<u32> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatSummaryModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(NavalCombatSummaryModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatSummaryInputs {
    pub recent_join_penalty: i64,
    pub escort_bonus: i64,
    pub countries: Vec<u32>,
    pub secondary_countries: Vec<u32>,
    pub modifiers: [[NavalCombatSummaryModifier; 5]; 5],
    pub modifier_metadata: [[u32; 2]; 5],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}
value_struct!(NavalCombatSummaryInputs { recent_join_penalty: i64, escort_bonus: i64, countries: Vec<u32>, secondary_countries: Vec<u32>, modifiers: [[NavalCombatSummaryModifier; 5]; 5], modifier_metadata: [[u32; 2]; 5], feature_flags: u32, eligibility_callback: u8 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatAirEfficiencyModifier {
    pub raw: i64,
    pub present: u8,
}
value_struct!(NavalCombatAirEfficiencyModifier {
    raw: i64,
    present: u8
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatAirEfficiency {
    pub source: u8,
    pub pool_type_flags: u64,
    pub mission: i32,
    pub container_present: u8,
    pub base_crowding: i64,
    pub night: i64,
    pub modifiers: [NavalCombatAirEfficiencyModifier; 3],
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatPlane {
    pub wing_types: Vec<u32>,
    pub source: u8,
    pub group_index: u32,
    pub index: u32,
    pub carrier_slot: i32,
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub units: Vec<u64>,
}
value_struct!(NavalCombatSideDetails { damage_history: [[i64; 6]; 6], original_health: i64, total_damage: i64, damage_by_type: Vec<NavalDamageSource>, commander: u64, engaged: u8, retreating: u8, capital_screening: i64, carrier_screening: i64, positioning: i64, anti_air: i64, planes: Vec<NavalCombatPlane>, summary: NavalCombatSummaryInputs, commander_inputs: Option<NavalCommanderInputs>, units: Vec<u64> });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalDamageSource {
    pub token: u32,
    pub damage: i64,
}
value_struct!(NavalDamageSource {
    token: u32,
    damage: i64
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandBorderWarSide {
    pub country: i32,
    pub state: i32,
    pub win_event: Vec<u8>,
    pub lose_event: Vec<u8>,
}
value_struct!(LandBorderWarSide { country: i32, state: i32, win_event: Vec<u8>, lose_event: Vec<u8> });
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandBorderWarInputs {
    pub transfer_state: u8,
    pub sides: [LandBorderWarSide; 2],
    pub combat_width: Recorded<i64>,
}
value_struct!(LandBorderWarInputs {
    transfer_state: u8,
    sides: [LandBorderWarSide; 2]
} appended LandBorderWarInputs {
    combat_width: Recorded<i64>
});

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    pub side_inputs: Recorded<[LandCombatSideInputs; 2]>,
}
typed_codec!(LandCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (province): U32, 1 => (phase): I32, 2 => (width_factor): I64, 3 => (attacker_bonus): I64, 4 => (defender_bonus): I64, 5 => (stored_terrain_valid): U8, 6 => (stored_terrain_name): Bytes, 7 => (sides): Opaque, 8 => (tactic_inputs): Opaque, 9 => (border_war): Opaque, 10 => (side_inputs): Opaque }, |row| { let _ = row; Ok(()) });

impl LandCombatDetailsRow {
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatMemberValues {
    pub naming: NameOperands,
    pub strength_combat_factor: i64,
    pub fuel_inputs: [i64; 2],
    pub stat_values: [i64; 5],
    pub armour: i64,
    pub piercing: i64,
    pub air_attack: i64,
    pub reconnaissance: i64,
    pub initiative: i64,
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
impl OpaqueValue for LandCombatMemberValues {}

impl LandCombatMemberValues {
    pub const STATS: [usize; 5] = [11, 12, 20, 9, 10];
    pub const ARMOUR: usize = 13;
    pub const PIERCING: usize = 14;
    pub const RECONNAISSANCE: usize = 3;
    pub const INITIATIVE: usize = 15;

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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatDivisionRow {
    pub combat: u64,
    pub side: u8,
    pub ordinal: u32,
    pub role: u8,
    pub province: u32,
    pub division: u64,
    pub commander: u64,
    pub scales: [i64; 4],
    pub context: LandDivisionContext,
    pub width_terms: Vec<LandDivisionWidthTerm>,
    pub direction_group: i32,
    pub exile: Option<LandCombatExileInputs>,
    pub categories: u64,
    pub reinforcement_speed: Option<i64>,
    pub own: Option<LandCombatMemberValues>,
}
impl LandCombatDivisionRow {
    pub fn values(
        &self,
        panel: Option<&DivisionPanelStatsRow>,
        status: Option<&DivisionStatusRow>,
        movement: Option<&DivisionMovementRow>,
    ) -> Option<LandCombatMemberValues> {
        Self::values_of(self.own.as_ref(), || LandCombatMemberValues::joined(panel?, status?, movement?))
    }

    pub fn values_of(
        own: Option<&LandCombatMemberValues>,
        joined: impl FnOnce() -> Option<LandCombatMemberValues>,
    ) -> Option<LandCombatMemberValues> {
        if let Some(own) = own {
            return Some(own.clone());
        }
        joined()
    }
    pub const SPEED_SHARE_MAXIMUM: i64 = 200_000;
}

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
 { 0 => (role): U8, 1 => (province): U32, 2 => (division): U64, 3 => (commander): U64, 4 => (scales[0]): I64, 5 => (scales[1]): I64, 6 => (scales[2]): I64, 7 => (scales[3]): I64, 8 => (context): Opaque, 9 => (width_terms): Opaque, 10 => (direction_group): I32, 11 => (exile): Opaque, 12 => (categories): U64, 13 => (reinforcement_speed): OptionalI64, 14 => (own): Opaque }, |row, mask| { if row.side > 1 { return Err(invalid("invalid combat side")); }
    if mask & (1_u64 << 13) != 0 && matches!(row.reinforcement_speed, Some(share) if !(0..=LandCombatDivisionRow::SPEED_SHARE_MAXIMUM).contains(&share)) { return Err(invalid("reserve speed share outside reinforcement bounds")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatModifierRow {
    pub combat: u64,
    pub side: u8,
    pub border_values: Option<[i64; 3]>,
    pub divisions: Retired,
}
typed_codec!(LandCombatModifierRow, 65, |row| (row.combat as u128) << 1 | row.side as u128,
 |key| Self { combat: (key >> 1) as u64, side: (key & 1) as u8, ..Default::default() },
 { 0 => (border_values): Opaque, 1 => (divisions): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatDivisionModifierRow {
    pub combat: u64,
    pub side: u8,
    pub ordinal: u32,
    pub entries: Vec<CombatModifierValue>,
    pub division: Option<u64>,
}
impl LandCombatDivisionModifierRow {
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct LandCombatSideRow {
    pub combat: u64,
    pub indicator: u8,
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
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct CombatEngagementRefreshRow {
    pub kind: u8,
    pub target: u64,
    pub side: u8,
    pub viewer: u32,
    pub triggers: u8,
}

impl CombatEngagementRefreshRow {
    pub const LAND_SIDE: u8 = 0;
    pub const NAVAL_SIDE: u8 = 1;
    pub const CONTROLLER_TARGET: u8 = 2;
    pub const LOCAL_ALL_SIDES: u8 = 3;
    pub const TRIGGER_RECORDED: u8 = 1;
    pub const TRIGGER_UNITS: u8 = 2;
    pub const TRIGGER_CACHE_WRITE: u8 = 4;
}

typed_codec!(
    CombatEngagementRefreshRow,
    99,
    |row| {
        (((row.kind & 3) as u128) << 97)
            | ((row.target as u128) << 33)
            | (((row.side & 1) as u128) << 32)
            | (row.viewer as u128)
    },
    |key| Self {
        kind: (key >> 97) as u8,
        target: (key >> 33) as u64,
        side: ((key >> 32) & 1) as u8,
        viewer: key as u32,
        ..Default::default()
    },
    { 0 => (triggers): U8 },
    |row, mask| {
        match row.kind {
            CombatEngagementRefreshRow::LAND_SIDE | CombatEngagementRefreshRow::NAVAL_SIDE => {
                if row.target == 0 {
                    return Err(invalid("invalid combat engagement refresh combat handle"));
                }
                if row.side > 1 {
                    return Err(invalid("invalid combat engagement refresh side"));
                }
                if row.viewer > i32::MAX as u32 {
                    return Err(invalid("invalid combat engagement refresh side viewer"));
                }
                if mask & 1 != 0 && (row.triggers == 0 || (row.triggers & !3) != 0) {
                    return Err(invalid("invalid combat engagement refresh triggers"));
                }
            }
            CombatEngagementRefreshRow::CONTROLLER_TARGET => {
                if row.side != 0 {
                    return Err(invalid("invalid combat engagement refresh side for kind"));
                }
                if row.viewer != 0 {
                    return Err(invalid("invalid combat engagement refresh viewer for kind"));
                }
                if row.target == 0 || row.target > i32::MAX as u64 {
                    return Err(invalid("invalid combat engagement refresh controller target"));
                }
                if mask & 1 != 0 && row.triggers != CombatEngagementRefreshRow::TRIGGER_RECORDED {
                    return Err(invalid("invalid combat engagement refresh triggers for kind"));
                }
            }
            CombatEngagementRefreshRow::LOCAL_ALL_SIDES => {
                if row.side != 0 {
                    return Err(invalid("invalid combat engagement refresh side for kind"));
                }
                if row.viewer != 0 {
                    return Err(invalid("invalid combat engagement refresh viewer for kind"));
                }
                if row.target > i32::MAX as u64 {
                    return Err(invalid("invalid combat engagement refresh local viewer"));
                }
                if mask & 1 != 0
                    && row.triggers != CombatEngagementRefreshRow::TRIGGER_RECORDED
                    && row.triggers
                        != (CombatEngagementRefreshRow::TRIGGER_RECORDED
                            | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE)
                {
                    return Err(invalid("invalid combat engagement refresh triggers for kind"));
                }
            }
            _ => return Err(invalid("invalid combat engagement refresh kind")),
        }
        Ok(())
    }
);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatDetailsRow {
    pub combat: u64,
    pub province: u32,
    pub day: i32,
    pub duration: i32,
    pub progress: i64,
    pub sides: [NavalCombatSideDetails; 2],
    pub custom_sea: Option<Vec<u8>>,
    pub custom_sea_error: u8,
}
typed_codec!(NavalCombatDetailsRow, 64, |row| row.combat as u128,
 |key| Self { combat: key as u64, ..Default::default() },
 { 0 => (province): U32, 1 => (day): I32, 2 => (duration): I32, 3 => (progress): I64, 4 => (sides): Opaque, 5 => (custom_sea): Opaque, 6 => (custom_sea_error): U8 }, |row, mask| {
    if mask & (1_u64 << 5) != 0 && row.custom_sea.as_ref().is_some_and(|sea| sea.len() % 2 != 0) { return Err(invalid("odd custom sea i16 bytes")); }
    if mask & (1_u64 << 6) != 0 && row.custom_sea_error > 1 { return Err(invalid("custom sea capture status conflicts with values")); }
    if mask & ((1_u64 << 5) | (1_u64 << 6)) == ((1_u64 << 5) | (1_u64 << 6)) && (row.custom_sea.is_some() == (row.custom_sea_error != 0)) { return Err(invalid("custom sea capture status conflicts with values")); }
    Ok(())
 });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatAirBaseWingHandlesRow {
    pub base: u64,
    pub wings: Vec<u64>,
}
typed_codec!(NavalCombatAirBaseWingHandlesRow, 64, |row| row.base as u128,
 |key| Self { base: key as u64, ..Default::default() },
 { 0 => (wings): U64List }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
typed_codec!(NavalCombatShipRow, 97, |row| (row.combat as u128) << 33 | (row.side as u128) << 32 | row.battle_slot as u128,
 |key| Self { combat: (key >> 33) as u64, side: ((key >> 32) & 1) as u8, battle_slot: key as u32, ..Default::default() },
 { 0 => (ship): U64, 1 => (owner): U32, 2 => (state): U32, 3 => (member_source): U8, 4 => (group_index): U32, 5 => (member_index): U32, 6 => (convoy): U64, 7 => (subunit_index): I32, 8 => (formation_type): I32, 9 => (build_cost_ic): I64, 10 => (cached_strength_ratio): I64, 11 => (pride): U8, 12 => (is_convoy): U8, 13 => (name): Bytes, 14 => (type_name): Bytes, 15 => (sprite): Bytes, 16 => (sunk_by): Bytes, 17 => (variant): U64, 18 => (arrival_hours): I32, 19 => (received_criticals): I32, 20 => (evacuation_state): I32, 21 => (detection_state): I32, 22 => (escape_progress): I64, 23 => (escape_speed_base): I64, 24 => (last_target): Opaque, 25 => (received_damage): Opaque, 26 => (live_ship): U8, 27 => (live_convoy): U8, 28 => (strength): I64, 29 => (organisation): I64, 30 => (base_organisation): I64, 31 => (maximum_hit_points): I64, 32 => (formation_flags): U64, 33 => (weapon_context): Opaque, 34 => (damage_entries): Opaque, 35 => (air_hits): Opaque, 36 => (naval_hits): Opaque }, |row| { let _ = row; if row.side > 1 { return Err(invalid("invalid combat side")); } Ok(()) });

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct NavalCombatResultDetailsRow {
    pub result: u64,
    pub sides: [NavalResultSideDetails; 2],
}
typed_codec!(NavalCombatResultDetailsRow, 64, |row| row.result as u128,
 |key| Self { result: key as u64, ..Default::default() },
 { 0 => (sides): Opaque }, |row| { let _ = row; Ok(()) });

opaque_value!(LandCombatTacticInputs);
impl OpaqueValue for LandBorderWarInputs {}
opaque_value!(LandDivisionContext);
opaque_value!(NavalCombatLastTarget);
opaque_value!(NavalShipWeaponContext);
impl OpaqueValue for LandCombatExileInputs {}
impl OpaqueValue for LandDivisionWidthTerm {}
impl OpaqueValue for NavalCombatDamage {}
impl OpaqueValue for NavalCombatHit {}
impl OpaqueValue for LandCombatSideDetails {}
impl OpaqueValue for LandCombatSideInputs {}
impl OpaqueValue for NavalCombatSideDetails {}
impl OpaqueValue for NavalResultSideDetails {}
impl OpaqueValue for NavalCombatAirHit {}

replay_rows::structure!(LandCombatSideDetails {
    committed: Vec<u64>, reserves: Vec<u64>, tactic_id: i32, tactic_kind: u8,
    tactic_counter_id: u32, tactic_token: Vec<u8>, air_counts: Vec<i32>,
    anti_air_attack: RetiredKept<i64>, air_kills: u32, damage: [i64; 2],
    ground_damage: RetiredNested<[i64; 2]>, prevented_damage: RetiredKept<[i64; 2]>,
    organisation_loss_sum: i64, organisation_loss_count: u32, commander: u64,
    commander_skill: i32, commander_category: i32, commander_country: u32,
    commander_experience: RetiredNested<i64>, commander_name_token: Vec<u8>,
    commander_description_token: RetiredNested<Vec<u8>>, commander_formation: u64,
    presentation: CombatCommanderPresentation, traits: Vec<LandCombatCommanderTrait>,
    timed_traits: RetiredNested<Vec<LandCombatCommanderTimedTrait>>,
    learning_traits: RetiredNested<Vec<LandCombatCommanderLearningTrait>>,
});

#[cfg(test)]
mod combat_engagement_refresh_tests {
    use super::*;

    #[test]
    fn rejects_malformed_combat_engagement_refresh_rows_on_decode() {
        for bad in [
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::NAVAL_SIDE,
                target: 0,
                side: 1,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 7,
                side: 0,
                viewer: i32::MAX as u32 + 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: i32::MAX as u64 + 1,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: i32::MAX as u64 + 1,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 0,
                viewer: 1,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_UNITS,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                    | CombatEngagementRefreshRow::TRIGGER_UNITS
                    | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LAND_SIDE,
                target: 7,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                    | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::CONTROLLER_TARGET,
                target: 7,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                    | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            },
        ] {
            assert!(bad.check_selected_fields(1).is_err(), "{bad:?}");
        }
        for good in [
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 0,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED,
            },
            CombatEngagementRefreshRow {
                kind: CombatEngagementRefreshRow::LOCAL_ALL_SIDES,
                target: 7,
                side: 0,
                viewer: 0,
                triggers: CombatEngagementRefreshRow::TRIGGER_RECORDED
                    | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE,
            },
        ] {
            assert!(good.check_selected_fields(1).is_ok(), "{good:?}");
        }
    }
}
