//! Fields shared by relation subclasses.

use crate::{
    cair_base_access_relation, cattache_relation, cboost_party_popularity_relation,
    cdocking_rights_relation, cembargo_relation, cequipment_purchase_relation, cguarantee_relation,
    cimprove_relations_relation, clend_lease_relation, clicence_production_access_relation,
    cmarket_access_rights_relation, cmilitary_access_relation, cnaval_blockade_relation,
    cnon_aggression_pact_relation, coffer_air_base_access_relation, coffer_docking_rights_relation,
    coffer_military_access_relation, csubject_relation, cwar_relation,
};

/// Stored relation type identifier.
pub const TYPE_ID: usize = 0x08;
/// War-relation type used by world-tension policy caps.
pub const TENSION_CAP_TYPE: u32 = 0x380a;
/// First country-link reference, u32.
pub const FIRST_COUNTRY: usize = 0x10;
/// Second country-link reference, u32.
pub const SECOND_COUNTRY: usize = 0x14;
/// Raw start hour, i32.
pub const START_HOUR: usize = 0x20;
/// Embedded start date with signed raw hour.
pub const START_DATE: usize = 0x18;
/// Raw end hour, i32.
pub const END_HOUR: usize = 0x38;
/// Cancellation byte; nonzero replaces start-date serialization with cancellation.
pub const CANCEL: usize = 0x48;
/// CRelation retired byte; native relation predicates exclude every nonzero value from the active set.
pub const RETIRED: usize = 0x49;

/// Base relation constructor installing vtable and passed type.
pub const CONSTRUCTOR_RVA: usize = 0x00d7_9390;

/// Supported relation subclasses as primary vtable RVA and type-word pairs.
/// War and subject relations have separate recorded tables but remain admitted
/// here so unclassified vtables produce explicit failures.
pub const CLASSES: [(usize, u32); 19] = [
    (
        cnaval_blockade_relation::VTABLE_RVA,
        cnaval_blockade_relation::TYPE,
    ),
    (
        cair_base_access_relation::VTABLE_RVA,
        cair_base_access_relation::TYPE,
    ),
    (
        coffer_air_base_access_relation::VTABLE_RVA,
        coffer_air_base_access_relation::TYPE,
    ),
    (cguarantee_relation::VTABLE_RVA, cguarantee_relation::TYPE),
    (
        cmilitary_access_relation::VTABLE_RVA,
        cmilitary_access_relation::TYPE,
    ),
    (
        cimprove_relations_relation::VTABLE_RVA,
        cimprove_relations_relation::TYPE,
    ),
    (
        cnon_aggression_pact_relation::VTABLE_RVA,
        cnon_aggression_pact_relation::TYPE,
    ),
    (
        coffer_military_access_relation::VTABLE_RVA,
        coffer_military_access_relation::TYPE,
    ),
    (csubject_relation::VTABLE_RVA, csubject_relation::TYPE_VALUE),
    (
        cboost_party_popularity_relation::VTABLE_RVA,
        cboost_party_popularity_relation::TYPE,
    ),
    (clend_lease_relation::VTABLE_RVA, clend_lease_relation::TYPE),
    (cembargo_relation::VTABLE_RVA, cembargo_relation::TYPE),
    (
        cequipment_purchase_relation::VTABLE_RVA,
        cequipment_purchase_relation::TYPE,
    ),
    (
        cmarket_access_rights_relation::VTABLE_RVA,
        cmarket_access_rights_relation::TYPE,
    ),
    (cwar_relation::VTABLE_RVA, cwar_relation::TYPE),
    (cattache_relation::VTABLE_RVA, cattache_relation::TYPE),
    (
        cdocking_rights_relation::VTABLE_RVA,
        cdocking_rights_relation::TYPE,
    ),
    (
        coffer_docking_rights_relation::VTABLE_RVA,
        coffer_docking_rights_relation::TYPE,
    ),
    (
        clicence_production_access_relation::VTABLE_RVA,
        clicence_production_access_relation::TYPE,
    ),
];

/// Type word paired with this supported primary vtable RVA.
pub fn class_type(vtable_rva: usize) -> Option<u32> {
    let mut index = 0;
    while index < CLASSES.len() {
        if CLASSES[index].0 == vtable_rva {
            return Some(CLASSES[index].1);
        }
        index += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_relation_class_has_a_distinct_vtable_and_type() {
        for (index, (vtable, type_word)) in CLASSES.iter().enumerate() {
            for (other_vtable, other_type) in &CLASSES[index + 1..] {
                assert_ne!(vtable, other_vtable);
                assert_ne!(type_word, other_type);
            }
            assert_eq!(class_type(*vtable), Some(*type_word));
        }
        assert_eq!(class_type(0), None);
        assert_eq!(
            class_type(cwar_relation::VTABLE_RVA),
            Some(TENSION_CAP_TYPE)
        );
    }
}
