use super::*;
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionVariableRow {
    pub faction: u64,
    pub ordinal: u32,
    pub name: Vec<u8>,
    pub value_raw: i64,
    pub seed: Option<u32>,
}
typed_codec!(FactionVariableRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128,
|key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
{ 0 => (name): Bytes, 1 => (value_raw): I64, 2 => (seed): OptionalU32 }, |row| {
    if (row.ordinal == 0) != row.seed.is_some() || (row.ordinal == 0 && (!row.name.is_empty() || row.value_raw != 0)) {
        return Err(invalid("invalid faction variable seed occurrence"));
    }
    Ok(())
});
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionRuleRow {
    pub faction: u64,
    pub ordinal: u32,
    pub token: u32,
}
typed_codec!(FactionRuleRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128,
|key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
{ 0 => (token): U32 }, |_row| Ok(()));
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionProgramRow {
    pub faction: u64,
    pub ordinal: u32,
    pub reference: u64,
}
typed_codec!(FactionProgramRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128,
|key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
{ 0 => (reference): U64 }, |_row| Ok(()));
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionManpowerRow {
    pub faction: u64,
    pub ordinal: u32,
    pub country: u32,
    pub value: i32,
}
typed_codec!(FactionManpowerRow, 96, |row| (row.faction as u128) << 32 | row.ordinal as u128,
|key| Self { faction: (key >> 32) as u64, ordinal: key as u32, ..Default::default() },
{ 0 => (country): U32, 1 => (value): I32 }, |_row| Ok(()));
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn faction_companion_rows_preserve_full_handles_signed_values_repeated_countries_and_null_programmes()
     {
        replay_rows::rows::round_trip(&FactionUnlockRow {
            faction: u64::MAX,
            research: 255,
            manpower: 0,
        });
        for ordinal in [0, u32::MAX] {
            replay_rows::rows::round_trip(&FactionRuleRow {
                faction: u64::MAX,
                ordinal,
                token: u32::MAX,
            });
            for reference in [0, u64::MAX] {
                replay_rows::rows::round_trip(&FactionProgramRow {
                    faction: u64::MAX,
                    ordinal,
                    reference,
                });
            }
            for value in [i32::MIN, 0, i32::MAX] {
                replay_rows::rows::round_trip(&FactionManpowerRow {
                    faction: u64::MAX,
                    ordinal,
                    country: u32::MAX,
                    value,
                });
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionUnlockRow {
    pub faction: u64,
    pub research: u8,
    pub manpower: u8,
}
typed_codec!(FactionUnlockRow, 64, |row| row.faction as u128,
|key| Self { faction: key as u64, ..Default::default() },
{ 0 => (research): U8, 1 => (manpower): U8 }, |_row| Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionAccountRow {
    pub contribution_subject_gain: Option<i64>,
    pub population_growth: Option<i64>,
    pub account: u32,
    pub kind: u32,
    pub ordinal: u32,
    pub statistic_raw: i64,
    pub country: u32,
    pub influence_numerator_raw: i64,
    pub influence_denominator_raw: i64,
    pub influence_refresh: u32,
    pub influence_faction: u64,
    pub influence_supplied: u8,
    pub influence_total_raw: i64,
    pub rank_derived: u8,
    pub rank_faction: u64,
    pub rank_input_raw: i64,
    pub rank_input_order: u32,
    pub rank_members: u32,
    pub projection_numerator_raw: i64,
    pub projection_denominator_raw: i64,
    pub projection_refresh: u32,
    pub influence_share_raw: i64,
    pub projection_share_raw: i64,
    pub projection_rank: i32,
    pub projection_derived: u8,
    pub initiative_raw: i64,
    pub initiative_product_raw: i64,
    pub initiative_steps: u32,
    pub contribution_raw: i64,
    pub contribution_gain_a_raw: i64,
    pub contribution_gain_b_raw: i64,
    pub previous_war_score_raw: i64,
    pub upgrade_groups: Vec<u32>,
    pub upgrade_tokens: Vec<u32>,
    pub completed_goals: Vec<u64>,
}
typed_codec!(FactionAccountRow, 96, |row| ((row.account as u128)<<64)|((row.kind as u128)<<32)|row.ordinal as u128,
|key| Self { account: (key>>64) as u32, kind:(key>>32) as u32, ordinal:key as u32, ..Default::default() },
{ 0 => (country): U32, 1 => (statistic_raw): I64, 2 => (initiative_raw): I64,
3 => (contribution_raw): I64, 4 => (contribution_gain_a_raw): I64, 5 => (contribution_gain_b_raw): I64,
6 => (previous_war_score_raw): I64, 7 => (upgrade_groups): U32List, 8 => (upgrade_tokens): U32List,
9 => (completed_goals): U64List, 10=>(influence_share_raw):I64,11=>(projection_share_raw):I64,12=>(projection_rank):I32,13=>(initiative_product_raw):I64,14=>(initiative_steps):U32,15=>(influence_numerator_raw):I64,16=>(influence_denominator_raw):I64,17=>(influence_refresh):U32,18=>(projection_numerator_raw):I64,19=>(projection_denominator_raw):I64,20=>(projection_refresh):U32,21=>(projection_derived):U8,22=>(influence_faction):U64,23=>(rank_derived):U8,24=>(rank_faction):U64,25=>(rank_input_raw):I64,26=>(rank_input_order):U32,27=>(rank_members):U32,28=>(influence_supplied):U8,29=>(influence_total_raw):I64,30=>(contribution_subject_gain):OptionalI64,31=>(population_growth):OptionalI64 }, |row| {
    if row.upgrade_groups.len() != row.upgrade_tokens.len() { return Err(invalid("faction account upgrade arrays differ")); }
    if row.completed_goals.iter().any(|value| *value > u32::MAX as u64 && *value != u64::MAX) { return Err(invalid("faction account goal token invalid")); }
    Ok(())
});
impl FactionAccountRow {
    pub fn initiative(&self) -> i64 {
        let increment = self.initiative_product_raw / 100_000;
        if self.initiative_steps == 0 {
            return self.initiative_raw;
        }
        let first = self.initiative_raw.wrapping_add(increment).max(0);
        let remaining = u64::from(self.initiative_steps - 1);
        if increment <= 0 {
            return (i128::from(first) + i128::from(increment) * i128::from(remaining)).max(0)
                as i64;
        }
        let before_reset = (i64::MAX - first) as u64 / increment as u64;
        if remaining <= before_reset {
            return first + (remaining * increment as u64) as i64;
        }
        let cycle = i64::MAX as u64 / increment as u64 + 1;
        (((remaining - before_reset - 1) % cycle) * increment as u64) as i64
    }
    pub fn normalized(numerator: i64, denominator: i64) -> i64 {
        let value = if denominator == 0 {
            0
        } else if (numerator.wrapping_add(i64::MAX - 1) as u64) < u64::MAX - 2 {
            numerator.wrapping_mul(100_000).wrapping_div(denominator)
        } else {
            u32::MAX as i64
        };
        value.clamp(-100_000, 100_000)
    }
    pub fn influence_share(&self) -> i64 {
        if self.influence_refresh == 0 {
            self.influence_share_raw
        } else {
            Self::normalized(self.influence_numerator_raw, self.influence_denominator_raw)
        }
    }
    pub fn projection_share(&self) -> i64 {
        if self.projection_refresh == 0 {
            self.projection_share_raw
        } else {
            Self::normalized(
                self.projection_numerator_raw,
                self.projection_denominator_raw,
            )
        }
    }
    pub fn projection(&self, rows: &[Self]) -> io::Result<Vec<i64>> {
        let mut result = Self::statistics(rows, self.account, 2).collect::<Vec<_>>();
        if self.projection_derived == 0 {
            return Ok(result);
        }
        let source = Self::statistics(rows, self.account, 3).collect::<Vec<_>>();
        let weights = Self::statistics(rows, 0, 4).collect::<Vec<_>>();
        if source.len() != 7 || weights.len() != 7 || result.len() < 4 {
            return Err(invalid("faction projection supplying operands incomplete"));
        }
        let count =
            |value: i64, weight: i64| value.wrapping_mul(weight).wrapping_mul(100_000) / 100_000;
        let derived = [
            (source[0] / 100_000).max(0).wrapping_add(source[1]),
            source[2] / 100_000,
            count(source[3], weights[3]).wrapping_add(count(source[4], weights[4])),
            count(source[5], weights[5]).wrapping_add(count(source[6], weights[6])),
        ];
        for (term, value) in derived.into_iter().enumerate() {
            if self.projection_derived & (1 << term) != 0 {
                result[term] = value;
            }
        }
        Ok(result)
    }
    pub fn rank(&self, rows: &[Self]) -> io::Result<i32> {
        if self.rank_derived == 0 {
            return Ok(self.projection_rank);
        }
        let peers = rows.iter().filter(|row| {
            row.kind == 0 && row.rank_faction == self.rank_faction && row.rank_derived != 0
        });
        let mut count = 0u32;
        let mut rank = 1i32;
        for row in peers {
            count = count
                .checked_add(1)
                .ok_or_else(|| invalid("faction rank cohort overflow"))?;
            if row.rank_input_raw > self.rank_input_raw
                || (row.rank_input_raw == self.rank_input_raw
                    && row.rank_input_order < self.rank_input_order)
            {
                rank = rank.wrapping_add(1);
            }
        }
        if count != self.rank_members {
            return Err(invalid("faction rank supplying cohort incomplete"));
        }
        Ok(rank)
    }
    pub fn statistics(rows: &[Self], account: u32, kind: u32) -> impl Iterator<Item = i64> + '_ {
        let first = rows.partition_point(|row| (row.account, row.kind) < (account, kind));
        rows[first..]
            .iter()
            .take_while(move |row| row.account == account && row.kind == kind)
            .map(|row| row.statistic_raw)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionResearchRow {
    pub faction: u64,
    pub ordinal: u32,
    pub scientist_base_raw: i64,
    pub countries: Vec<u32>,
    pub bonus_raw: i64,
}
typed_codec!(FactionResearchRow,96,|row|(row.faction as u128)<<32|row.ordinal as u128,
|key|Self{faction:(key>>32) as u64,ordinal:key as u32,..Default::default()},
{0=>(scientist_base_raw):I64,1=>(countries):U32List,2=>(bonus_raw):I64},|_row|Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionMinimumUpgradeRow {
    pub faction: u64,
    pub ordinal: u32,
    pub group: u32,
    pub token: u32,
}
typed_codec!(FactionMinimumUpgradeRow,96,|row|(row.faction as u128)<<32|row.ordinal as u128,
|key|Self{faction:(key>>32) as u64,ordinal:key as u32,..Default::default()},
{0=>(group):U32,1=>(token):U32},|_row|Ok(()));
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionDoctrineRow {
    pub faction: u64,
    pub ordinal: u32,
    pub token: u32,
}
typed_codec!(FactionDoctrineRow,96,|row|(row.faction as u128)<<32|row.ordinal as u128,
|key|Self{faction:(key>>32) as u64,ordinal:key as u32,..Default::default()},
{0=>(token):U32},|_row|Ok(()));

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionProjectionRow {
    pub faction: u64,
    pub projection_raw: i64,
    pub influence_raw: i64,
    pub effects_raw: i64,
    pub influence_phase: u64,
    pub influence_members: u32,
    pub influence_awards_raw: i64,
    pub rank_phase: u64,
    pub projection_phase: u64,
    pub influence_share_phase: u64,
    pub influence_accounts: Vec<u32>,
    pub rank_members: u32,
}
typed_codec!(FactionProjectionRow,64,|row|row.faction as u128,|key|Self{faction:key as u64,..Default::default()},
{0=>(projection_raw):I64,1=>(influence_raw):I64,2=>(effects_raw):I64,3=>(influence_phase):U64,4=>(influence_members):U32,5=>(influence_awards_raw):I64,6=>(rank_phase):U64,7=>(rank_members):U32,8=>(projection_phase):U64,9=>(influence_share_phase):U64,10=>(influence_accounts):U32List},|_row|Ok(()));
impl FactionProjectionRow {
    pub fn influence_total(&self, accounts: &[FactionAccountRow]) -> io::Result<i64> {
        if self.influence_phase == 0 {
            return Ok(self.influence_raw);
        }
        if self.influence_accounts.len() != self.influence_members as usize {
            return Err(invalid("faction influence supplying cohort incomplete"));
        }
        let mut total = 0i64;
        for &account in &self.influence_accounts {
            let at = accounts.partition_point(|row| (row.account, row.kind) < (account, 0));
            let row = accounts
                .get(at)
                .filter(|row| {
                    row.account == account
                        && row.kind == 0
                        && row.influence_faction == self.faction
                        && row.influence_supplied != 0
                })
                .ok_or_else(|| invalid("faction influence supplying account missing"))?;
            total = total.wrapping_add(row.influence_total_raw);
        }
        Ok(total.wrapping_add(self.influence_awards_raw))
    }
}
#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionResourceRow {
    pub faction: u64,
    pub ordinal: u32,
    pub token: u32,
    pub raw: i64,
}
typed_codec!(FactionResourceRow,96,|row|(row.faction as u128)<<32|row.ordinal as u128,
|key|Self{faction:(key>>32) as u64,ordinal:key as u32,..Default::default()},
{0=>(token):U32,1=>(raw):I64},|_row|Ok(()));

#[cfg(test)]
mod statistic_tests {
    use super::*;
    #[test]
    fn population_growth_changes_independently_of_subject_gain_and_other_account_terms() {
        let old = FactionAccountRow {
            account: 3,
            country: 5,
            contribution_subject_gain: Some(-17),
            population_growth: Some(25),
            ..Default::default()
        };
        let mut next = old.clone();
        next.population_growth = Some(-50);
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 31);
        assert!(delta.values.len() <= 9);
        assert_eq!(delta.apply(Some(old)).unwrap(), next);
        replay_rows::rows::round_trip(&next);
        let mut absent = next.clone();
        absent.population_growth = None;
        assert_eq!(absent.delta(Some(&next)).unwrap().unwrap().mask, 1 << 31);
        replay_rows::rows::round_trip(&absent);
    }
    #[test]
    fn one_statistic_change_encodes_only_its_scalar_and_carries_other_terms() {
        let old = FactionAccountRow {
            account: 5000,
            kind: 1,
            ordinal: 4,
            statistic_raw: 17,
            ..Default::default()
        };
        let mut next = old.clone();
        next.statistic_raw = 19;
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 1);
        assert!(delta.values.len() <= 8);
        assert_eq!(delta.apply(Some(old)).unwrap(), next);
        let sibling = FactionAccountRow {
            account: 5000,
            kind: 1,
            ordinal: 3,
            statistic_raw: -7,
            ..Default::default()
        };
        assert!(sibling.delta(Some(&sibling)).unwrap().is_none());
    }
    #[test]
    fn research_bonus_occurrences_carry_independently() {
        let old = FactionResearchRow {
            faction: 7,
            ordinal: 9,
            bonus_raw: 17,
            ..Default::default()
        };
        let mut next = old.clone();
        next.bonus_raw = 19;
        let delta = next.delta(Some(&old)).unwrap().unwrap();
        assert_eq!(delta.mask, 1 << 2);
        assert!(delta.values.len() <= 8);
        assert_eq!(delta.apply(Some(old)).unwrap(), next);
    }
    #[test]
    fn unchanged_member_operands_carry_when_only_owner_refresh_boundaries_advance() {
        let member = FactionAccountRow {
            account: 17,
            rank_derived: 1,
            influence_supplied: 1,
            influence_refresh: 1,
            projection_refresh: 1,
            ..Default::default()
        };
        assert!(member.delta(Some(&member)).unwrap().is_none());
        let old = FactionProjectionRow {
            faction: 7,
            influence_phase: 1,
            rank_phase: 2,
            projection_phase: 3,
            influence_share_phase: 4,
            influence_accounts: vec![17],
            ..Default::default()
        };
        let mut next = old.clone();
        next.influence_phase = 5;
        next.rank_phase = 6;
        next.projection_phase = 7;
        next.influence_share_phase = 8;
        assert_eq!(
            next.delta(Some(&old)).unwrap().unwrap().mask,
            (1 << 3) | (1 << 6) | (1 << 8) | (1 << 9)
        );
    }
    #[test]
    fn passive_initiative_derives_truncation_clamp_overflow_and_sparse_resets() {
        for base in [i64::MIN, -3, 0, 7, i64::MAX - 4, i64::MAX] {
            for product in [i64::MIN, -300_001, -1, 0, 1, 200_001, i64::MAX] {
                for steps in [0, 1, 2, 17, 97] {
                    let mut expected = base;
                    for _ in 0..steps {
                        expected = expected.wrapping_add(product / 100_000).max(0);
                    }
                    let row = FactionAccountRow {
                        initiative_raw: base,
                        initiative_product_raw: product,
                        initiative_steps: steps,
                        ..Default::default()
                    };
                    assert_eq!(row.initiative(), expected, "{base}/{product}/{steps}");
                }
            }
        }
        let row = FactionAccountRow {
            initiative_raw: 7,
            initiative_product_raw: 0,
            initiative_steps: u32::MAX,
            ..Default::default()
        };
        assert_eq!(row.initiative(), 7);
    }
    #[test]
    fn shares_keep_supplying_versions_when_current_influence_changes_again() {
        let row = FactionAccountRow {
            influence_refresh: 7,
            influence_numerator_raw: 20,
            influence_denominator_raw: 80,
            projection_refresh: 8,
            projection_numerator_raw: -1,
            projection_denominator_raw: 4,
            ..Default::default()
        };
        assert_eq!(row.influence_share(), 25_000);
        assert_eq!(row.projection_share(), -25_000);
        assert_eq!(FactionAccountRow::normalized(4, 0), 0);
        assert_eq!(FactionAccountRow::normalized(i64::MAX, 1), 100_000);
        assert_eq!(FactionAccountRow::normalized(-20, 1), -100_000);
    }
    #[test]
    fn projection_uses_phase_operands_independent_of_current_sources_and_preserves_extra_terms() {
        let mut rows = vec![FactionAccountRow {
            projection_derived: 15,
            ..Default::default()
        }];
        for (kind, values) in [
            (2, vec![0, 0, 0, 0, 19]),
            (3, vec![15_000_050_000, 300_000, 1_000_000, 2, 5, 7, 11]),
            (
                4,
                vec![50_000, 100_000, 2, 100_000, 100_000, 100_000, 100_000],
            ),
        ] {
            for (ordinal, statistic_raw) in values.into_iter().enumerate() {
                rows.push(FactionAccountRow {
                    kind,
                    ordinal: ordinal as u32,
                    statistic_raw,
                    ..Default::default()
                });
            }
        }
        assert_eq!(
            rows[0].projection(&rows).unwrap(),
            [450_000, 10, 700_000, 1_800_000, 19]
        );
        rows.pop();
        assert!(rows[0].projection(&rows).is_err());
    }
    #[test]
    fn ranks_use_refresh_cohort_inputs_and_native_member_order_for_ties() {
        let mut rows = (0..3)
            .map(|account| FactionAccountRow {
                account,
                rank_faction: 7,
                rank_derived: 1,
                rank_members: 3,
                rank_input_raw: if account == 2 { 30 } else { 20 },
                rank_input_order: account,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        assert_eq!(rows[0].rank(&rows).unwrap(), 2);
        assert_eq!(rows[1].rank(&rows).unwrap(), 3);
        assert_eq!(rows[2].rank(&rows).unwrap(), 1);
        rows[2].rank_derived = 0;
        assert!(rows[0].rank(&rows).is_err());
        rows[0].rank_derived = 0;
        rows[0].projection_rank = 17;
        assert_eq!(rows[0].rank(&rows).unwrap(), 17);
    }
    #[test]
    fn influence_total_uses_refresh_sum_plus_awards_without_summing_later_share_inputs() {
        let accounts = vec![
            FactionAccountRow {
                influence_faction: 7,
                influence_supplied: 1,
                influence_total_raw: 20,
                influence_numerator_raw: 99,
                ..Default::default()
            },
            FactionAccountRow {
                account: 1,
                influence_faction: 7,
                influence_supplied: 1,
                influence_total_raw: 30,
                ..Default::default()
            },
        ];
        let mut row = FactionProjectionRow {
            faction: 7,
            influence_phase: 9,
            influence_members: 2,
            influence_accounts: vec![0, 1],
            influence_awards_raw: 8,
            ..Default::default()
        };
        assert_eq!(row.influence_total(&accounts).unwrap(), 58);
        row.influence_accounts[1] = 9;
        assert!(row.influence_total(&accounts).is_err());
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct FactionAdvisorModifierRow {
    pub faction: u64,
    pub slot: u32,
    pub ordinal: u32,
    pub kind: u32,
    pub country: u32,
    pub character: Option<u64>,
    pub date: i32,
    pub date_present: u8,
    pub name: Vec<u8>,
    pub modifier: u32,
    pub raw: i64,
}
typed_codec!(FactionAdvisorModifierRow,128,|row|((row.faction as u128)<<64)|((row.slot as u128)<<32)|row.ordinal as u128,
|key|Self{faction:(key>>64) as u64,slot:(key>>32) as u32,ordinal:key as u32,..Default::default()},
{0=>(kind):U32,1=>(country):U32,2=>(character):OptionalU64,3=>(date):I32,4=>(name):Bytes,5=>(modifier):U32,6=>(raw):I64,7=>(date_present):U8},|_row|Ok(()));
