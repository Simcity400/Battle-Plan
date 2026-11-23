pub use engine_reference_values::native_values::naval_modifiers::KEYS;
pub use game_model::naval_modifier_trees::{Kind, SideSource};

use crate::{
    ModifierBreakdownNode, NavalCombatSideDetails, NavalModifierTreeRow, NavalShipModifierSource,
    NavalShipWeaponContext,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Owner {
    pub kind: Kind,
    pub owner: u64,
    pub sub: u32,
}

impl Owner {
    pub const fn new(kind: Kind, owner: u64, sub: u32) -> Self {
        Self { kind, owner, sub }
    }

    pub const fn key(self) -> u128 {
        (self.kind as u8 as u128) << 96 | (self.owner as u128) << 32 | self.sub as u128
    }
}

pub fn ship_source(
    ship: u64,
    context: &NavalShipWeaponContext,
    source: &NavalShipModifierSource,
) -> Option<Owner> {
    game_model::naval_modifier_trees::ship_owner(
        source.scope,
        source.index,
        source.selector,
        source.reference,
        ship,
        context.task_force,
        context.country,
        context.commander,
        context.ship_class,
        context.subunit,
    )
    .map(|(kind, owner, sub)| Owner::new(kind, owner, sub))
}

pub fn side_source(
    combat: u64,
    side: &NavalCombatSideDetails,
    opposing: &NavalCombatSideDetails,
    source: SideSource,
) -> Option<Owner> {
    game_model::naval_modifier_trees::side_owner(
        source,
        combat,
        side.summary.countries.first().copied(),
        side.commander,
        opposing.commander,
    )
    .map(|(kind, owner, sub)| Owner::new(kind, owner, sub))
}

pub fn side_sources(key: u32) -> &'static [SideSource] {
    match key {
        0x1b3 => &[SideSource::Country],
        0x5a => &[
            SideSource::Country,
            SideSource::Weather,
            SideSource::Terrain,
            SideSource::OpposingCommander,
        ],
        _ => &[
            SideSource::Country,
            SideSource::Weather,
            SideSource::Terrain,
            SideSource::Commander,
        ],
    }
}

#[derive(Clone, Copy)]
pub struct Trees<'a> {
    rows: &'a [NavalModifierTreeRow],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tree<'a> {
    Nodes(&'a [ModifierBreakdownNode]),
    Empty,
}

impl<'a> Tree<'a> {
    pub fn root(self) -> i64 {
        match self {
            Self::Nodes(nodes) => nodes.first().map_or(0, |node| node.value),
            Self::Empty => 0,
        }
    }

    pub fn nodes(self) -> &'a [ModifierBreakdownNode] {
        match self {
            Self::Nodes(nodes) => nodes,
            Self::Empty => &[],
        }
    }
}

impl<'a> Trees<'a> {
    pub fn new(rows: &'a [NavalModifierTreeRow]) -> Option<Self> {
        rows.windows(2)
            .all(|pair| pair[0].key() < pair[1].key())
            .then_some(Self { rows })
    }

    pub fn row(&self, owner: Owner) -> Option<&'a NavalModifierTreeRow> {
        let key = owner.key();
        self.rows
            .binary_search_by_key(&key, NavalModifierTreeRow::key)
            .ok()
            .map(|index| &self.rows[index])
    }

    pub fn tree(&self, owner: Owner, key: u32) -> Tree<'a> {
        self.row(owner)
            .and_then(|row| row.trees.iter().find(|tree| tree.key == key))
            .map_or(Tree::Empty, |tree| Tree::Nodes(&tree.nodes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NavalCombatSummaryInputs, NavalModifierKeyTree};

    fn side(country: Option<u32>, commander: u64) -> NavalCombatSideDetails {
        NavalCombatSideDetails {
            commander,
            summary: NavalCombatSummaryInputs {
                countries: country.into_iter().collect(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn ship_sources_join_their_recorded_owners() {
        let context = NavalShipWeaponContext {
            task_force: 77,
            country: 5,
            commander: 99,
            ship_class: 3,
            subunit: 12,
            ..Default::default()
        };
        let source = |scope, index, selector, reference| NavalShipModifierSource {
            scope,
            index,
            selector,
            reference,
            ..Default::default()
        };
        assert_eq!(
            ship_source(41, &context, &source(0, 0, 0, 0)),
            Some(Owner::new(Kind::Country, 5, 0))
        );
        assert_eq!(
            ship_source(41, &context, &source(4, 2, 0x2a, 0)),
            Some(Owner::new(Kind::Doctrine, 77, 0x2a))
        );
        assert_eq!(
            ship_source(41, &context, &source(5, 0, 0, 0)),
            Some(Owner::new(Kind::Terrain, 77, 3))
        );
        assert_eq!(
            ship_source(41, &context, &source(10, 0, 12, 0)),
            Some(Owner::new(Kind::CommanderClass, 99, 12))
        );
        assert_eq!(
            ship_source(41, &context, &source(11, 4, 0, 0)),
            Some(Owner::new(Kind::Critical, 41, 4))
        );
        assert_eq!(
            ship_source(41, &context, &source(14, 1, 0, 0x55)),
            Some(Owner::new(Kind::Support2, 0x55, 0))
        );
        for scope in [16, 20, 24, 27, 31, 32] {
            assert_eq!(ship_source(41, &context, &source(scope, 0, 0, 0)), None);
        }
    }

    #[test]
    fn side_sources_skip_an_empty_side_and_a_missing_commander() {
        let own = side(Some(7), 0);
        let opposing = side(Some(8), 66);
        assert_eq!(
            side_source(1, &side(None, 5), &opposing, SideSource::Country),
            None
        );
        assert_eq!(
            side_source(1, &own, &opposing, SideSource::Country),
            Some(Owner::new(Kind::Country, 7, 0))
        );
        assert_eq!(side_source(1, &own, &opposing, SideSource::Commander), None);
        assert_eq!(
            side_source(1, &own, &opposing, SideSource::OpposingCommander),
            Some(Owner::new(Kind::Commander, 66, 0))
        );
        assert_eq!(
            side_source(1, &own, &opposing, SideSource::Weather),
            Some(Owner::new(Kind::SideWeather, 1, 0))
        );
        assert_eq!(side_sources(0x1b3), &[SideSource::Country]);
        assert_eq!(side_sources(0x5a)[3], SideSource::OpposingCommander);
    }

    #[test]
    fn a_reached_source_without_a_tree_is_empty_with_a_zero_root() {
        let owner = Owner::new(Kind::Commander, 66, 0);
        let rows = [NavalModifierTreeRow {
            kind: owner.kind as u8,
            owner: owner.owner,
            sub: owner.sub,
            trees: vec![NavalModifierKeyTree {
                key: 0x1b2,
                nodes: vec![ModifierBreakdownNode {
                    children: 0,
                    value: 5_000,
                    name: b"Trait".to_vec(),
                }],
            }],
        }];
        let trees = Trees::new(&rows).unwrap();
        assert_eq!(trees.tree(owner, 0x1b2).root(), 5_000);
        assert_eq!(trees.tree(owner, 0x5a), Tree::Empty);
        assert_eq!(trees.tree(Owner::new(Kind::Country, 1, 0), 0x1b2).root(), 0);
        assert_eq!(rows[0].key(), owner.key());
    }
}
