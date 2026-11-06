//! The runtime writers of the two naval hover trees the collector carries:
//! a country's modifier tree and a commander's (`naval_modifier_trees`, kinds
//! Country and Commander). Observing them tells the collector which trees to
//! reread; recording never calls them. Only commander sites are patched here.
//! The stability and research family owns country modifier recalculation;
//! the politics family owns appearance reset. Both forward their calls.
//!
//! Every site takes at most four integer or pointer arguments: country
//! recalculation and appearance reset, commander recalculation and HQ scaling,
//! and leader construction and destruction take one receiver; the copy
//! constructor also takes its source. Forwarding RCX, RDX, R8 and R9 and
//! returning RAX therefore preserves every argument and result.
//!
//! Country tree (`naval_modifier_trees::COUNTRY_TREE`, in the country's modifier
//! block, with the per-source children used by the breakdown formatter):
//! - Country modifier recalculation clears and rebuilds it in place. Eight
//!   helper paths, including dynamic modifier value recalculation, receive
//!   the block or tree only inside this call; every other add occurs there.
//!   Callers include commands, effects, the hourly pending mask, ability
//!   expiry, decision timeouts, daily dynamic modifiers and leader level-ups.
//! - Appearance reset clears it during dynamic reuse (civil war, dynamic
//!   country creation or reservation, and peace-conference participants),
//!   game runtime reset and country-file loading.
//! - Construction, destruction and country member loading are load-only;
//!   only country-file loading constructs countries. Load changes the epoch.
//!
//! The writer inventory covers every displacement into the country's
//! modifier block, formed block and tree pointers and the calls they reach,
//! header stores, stores through tree data pointers and destinations of
//! every accumulator mutator: filtered addition, clear, copy and move
//! assignment, child cloning, dynamic value recalculation and two further
//! mutators. Other country block and tree uses only read. Child cloning
//! reuses pooled children, so equal child pointers cannot certify an
//! unchanged tree.
//!
//! Commander tree (`naval_modifier_trees::COMMANDER_TREE`, the value
//! accumulator of the leader's modifier block, read by naval mission and ship
//! modifier calculations):
//! - A tail helper called only by leader modifier recalculation clears it,
//!   copies the leader's base accumulator and scales it.
//! - HQ modifier scaling copies the same accumulator with the HQ
//!   communication scale, from orders-group paths.
//! - Construction, copy construction and destruction revoke certificates, so
//!   reusing an address cannot keep a freed leader's certificate. The
//!   accumulator is not serialized; load rebuilds it and changes the epoch.
//!
//! The leader writer inventory covers the leader's modifier blocks from the
//! commander block on, and the data subobject used by constructors and
//! destruction. Pointers formed at block offsets reach mutators only through
//! the functions above.

pub use crate::advisor_mutations::MutationSite;

/// The first [`COUNTRY_SITES`] sites receive a country in `rcx`; the rest
/// receive the unit leader (full object). The collector's callbacks follow
/// this order.
pub const COUNTRY_SITES: usize = 2;

pub const MUTATIONS: [MutationSite; 7] = [
    MutationSite {
        name: "recalculate_modifiers",
        rva: 0x722930,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
            0x56, 0x41,
        ],
    },
    MutationSite {
        name: "reset_country_appearance",
        rva: 0x72a990,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
            0x20, 0x55,
        ],
    },
    MutationSite {
        name: "recalculate_leader_modifiers",
        rva: 0xc77590,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x57,
        ],
    },
    MutationSite {
        name: "update_hq_scaled_modifiers",
        rva: 0xc77be0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
            0x20, 0x57,
        ],
    },
    MutationSite {
        name: "construct_leader",
        rva: 0xc61da0,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48,
            0x83, 0xc1,
        ],
    },
    MutationSite {
        name: "copy_construct_leader",
        rva: 0xc61d00,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48,
            0x83, 0xc1,
        ],
    },
    MutationSite {
        name: "destroy_leader",
        rva: 0xc62e40,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xe3, 0xf6, 0xd6, 0x01, 0xc7,
            0x81, 0x7c,
        ],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_distinct_and_each_sentinel_starts_its_function() {
        let mut rvas: Vec<usize> = MUTATIONS.iter().map(|site| site.rva).collect();
        rvas.sort_unstable();
        rvas.dedup();
        assert_eq!(rvas.len(), MUTATIONS.len());
        // Every sentinel opens with a prologue store or push.
        for site in &MUTATIONS {
            assert!(matches!(site.sentinel[0], 0x48 | 0x40), "{}", site.name);
        }
        assert_eq!(MUTATIONS[..COUNTRY_SITES].len(), 2);
        assert_eq!(
            MUTATIONS[COUNTRY_SITES].name,
            "recalculate_leader_modifiers"
        );
    }
}
