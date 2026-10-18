//! Ordered raid launch eligibility over the recorded hour.

use super::{
    RaidAssets, TypeToken,
    script::{Eval, Scope, Scopes, World},
};
use installed_content_raids::raids::RaidType;
use replay_data::rows::tables;

/// Launch-trigger order:
///
/// - Type unlocked for the owner (`visible` and category unlock through
///   [`super::offers::category_admits_unlock`]), not used up (`fire_only_once`),
///   and `show_target`.
/// - Off cooldown and `available`.
/// - `launchable`.
/// - `launchable_from`, with the source country as ROOT.
///
/// Every trigger except the last uses owner scopes: owner as ROOT, target
/// country as FROM, and target state, province and leader as variables.
pub fn launch_triggers(
    assets: &RaidAssets,
    world: &World<'_>,
    owner: u32,
    kind: &RaidType,
    scope: &replay_data::rows::RaidScope,
    source_country: &Result<Option<u32>, String>,
) -> Eval {
    let target_country = match super::target_country(world, scope) {
        Ok(country) => country,
        Err(reason) => return Eval::unknown(reason),
    };
    let scopes = Scopes {
        cursor: None,
        root: Scope::Country(owner),
        this: Scope::Country(owner),
        prev: None,
        from: target_country.map(Scope::Country),
        target_state: super::offers::target_state(world, scope),
        target_province: [
            scope.province,
            if scope.building_type != 0 && scope.building_flags & 1 != 0 {
                scope.building_location
            } else {
                0
            },
            scope.leader_province,
        ]
        .into_iter()
        .find(|&id| id != 0),
        target_leader: (scope.leader_reference != 0).then_some(scope.leader_reference),
        unknown: Default::default(),
        bare: false,
    };
    world
        .evaluate(&kind.visible, Scopes::of(Scope::Country(owner)))
        .and(|| super::offers::category_admits_unlock(assets, world, owner, kind))
        .and(|| used_up(assets, world, owner, kind).negate())
        .and(|| world.evaluate(&kind.show_target, scopes))
        .and(|| on_cooldown(assets, world, owner, kind, scope).negate())
        .and(|| world.evaluate(&kind.available, scopes))
        .and(|| world.evaluate(&kind.launchable, scopes))
        .and(
            || match (world.has_children(&kind.launchable_from), source_country) {
                (Err(reason), _) => Eval::unknown(reason),
                (Ok(false), _) => Eval::True,
                (Ok(true), source_country) => match source_country {
                    Ok(Some(country)) => world.evaluate(
                        &kind.launchable_from,
                        Scopes {
                            root: Scope::Country(*country),
                            this: Scope::Country(*country),
                            ..scopes
                        },
                    ),
                    Ok(None) => Eval::unknown("the raid source has no country for launchable_from"),
                    Err(reason) => Eval::unknown(reason.clone()),
                },
            },
        )
}

/// `fire_only_once` and the type in the owner's recorded used types
/// (`RaidCountries.used_types`).
pub fn used_up(assets: &RaidAssets, world: &World<'_>, owner: u32, kind: &RaidType) -> Eval {
    if !kind.fire_only_once {
        return Eval::False;
    }
    let rows = match world.frame().complete::<tables::RaidCountries>() {
        Ok(rows) => rows,
        Err(error) => return Eval::unknown(error.to_string()),
    };
    let Some(status) = rows.iter().find(|row| row.canonical_slot == owner) else {
        return Eval::False;
    };
    let mut result = Eval::False;
    for &token in &status.used_types {
        result = result.or(|| match assets.raid_type(world, token) {
            Ok(TypeToken::Type(index)) => {
                Eval::from_bool(assets.catalog.types[index].name == kind.name)
            }
            Ok(TypeToken::Refused(_)) => Eval::False,
            Err(reason) => Eval::unknown(format!("a used raid type may be this one: {reason}")),
        });
    }
    result
}

/// Owner cooldown (`RaidCooldowns`) of this type whose building, province,
/// state and leader arms match the target.
pub fn on_cooldown(
    assets: &RaidAssets,
    world: &World<'_>,
    owner: u32,
    kind: &RaidType,
    scope: &replay_data::rows::RaidScope,
) -> Eval {
    let rows = match world.frame().complete::<tables::RaidCooldowns>() {
        Ok(rows) => rows,
        Err(error) => return Eval::unknown(error.to_string()),
    };
    let matches = |row: &replay_data::rows::RaidScope| {
        row.building_type == scope.building_type
            && (scope.building_type == 0
                || (row.building_location == scope.building_location
                    && row.building_flags == scope.building_flags))
            && row.province == scope.province
            && row.state == scope.state
            && row.leader_reference == scope.leader_reference
    };
    let mut result = Eval::False;
    for row in rows
        .iter()
        .filter(|row| row.owner_slot == owner && matches(&row.scope))
    {
        result = result.or(|| match assets.raid_type(world, row.raid_type) {
            Ok(TypeToken::Type(index)) => {
                Eval::from_bool(assets.catalog.types[index].name == kind.name)
            }
            Ok(TypeToken::Refused(at)) => {
                Eval::from_bool(assets.catalog.uncertain[at].name == kind.name)
            }
            Err(reason) => Eval::unknown(reason),
        });
    }
    result
}
