//! The runtime writers of per-country fields read hourly by Politics: capital
//! and original capital (`ccountry::CAPITAL`, `ORIGINAL_CAPITAL`);
//! political-status last election, ruling party, party array and count, active
//! idea array and count (`cpolitical_status::LAST_ELECTION`, `RULING_PARTY`,
//! `PARTIES`, `PARTY_COUNT`, `IDEAS`, `IDEA_COUNT`) and all their slots; and
//! party popularity (`cpolitical_party::POPULARITY`).
//! Observing them tells the collector when the walk may read differently;
//! recording never calls them.
//!
//! Writer coverage includes stores to these fields, loads of the country's
//! political status (`ccountry::POLITICAL_STATUS`) and its accessor followed
//! through registers and calls, and stores outside indexed functions:
//! - Capitals: the capital setter runs when state controller changes restore
//!   the original capital, when selecting a replacement, through state
//!   transfer and capital effects, civil war, peace conferences and capital
//!   change application. Appearance reset also writes them during dynamic
//!   reuse and game runtime reset; otherwise only construction and save load.
//! - The country's political-status pointer and status owner backlink
//!   (`cpolitical_status::COUNTRY`) are written only by constructors.
//!   Countries are reused, never rebuilt.
//! - Last election: setting politics, the daily update on election day,
//!   status clear, construction and save loading. Only appearance reset and
//!   load completion call the clear.
//! - Ruling party: forced selection from console puppet, subject relations
//!   and peace conferences; selection by ideology from setting politics,
//!   civil war and console; clear and party build, both called only by
//!   appearance reset and load completion; and post-save validation.
//! - Party array and slots: party build, clear, construction and destruction.
//! - Idea array and slots: one runtime replacement writer through two
//!   entry-jumping thunks, from effects, timed ideas, focuses, daily update,
//!   idea commands and console; also clear, construction and destruction.
//! - Popularity: the clamped setter's leading sign test and branch cannot be
//!   detoured. Daily politics and forced ruling-party selection call it and
//!   are hooked at entry. The political-party and popularities effects are
//!   observed at their calls to it. Popularity addition (effect and console)
//!   and the redistributing setter (civil war, dynamic country creation and
//!   the political-party effect) each loop inline over the country's parties.
//!   Party construction and save loading also write popularity. Boosting
//!   party popularity only stores an ideology on the relation; daily politics
//!   applies it.
//! - No writer runs on a worker thread; parallel daily bodies reach none.
//!
//! The politics capture family (`politics_sites`) owns appearance
//! reset and daily politics, forwarding them through [`FORWARDED`]. Save and
//! history readers, post-read validation and load completion run only inside
//! a load, invalidating every observation.
//!
//! Writers have two token domains ([`Domain`]): structure (capitals, last
//! election, ruling party, party and idea arrays) and popularity. Popularity
//! changes daily; structure rarely does. Daily politics always rewrites
//! popularity through the clamped setter. Its sole inline structure store
//! writes last election on election day; ruling-party and idea changes pass
//! through hooked writers. Forwarding therefore moves popularity always,
//! and structure only when the last-election value differs across the call.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// How a site is patched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Patch {
    /// A detour at the function's entry.
    Entry,
    /// A replaced five-byte CALL of [`SET_POPULARITY_RVA`].
    Call,
}

/// Which token domain a site's call moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    /// Capitals, last election, ruling party and the party and idea arrays.
    Structure,
    /// Party popularity.
    Popularity,
    /// Both: forced ruling-party selection redistributes popularity through
    /// the clamped setter; appearance reset rebuilds the parties.
    Both,
}

impl Domain {
    pub const fn structure(self) -> bool {
        matches!(self, Self::Structure | Self::Both)
    }
    pub const fn popularity(self) -> bool {
        matches!(self, Self::Popularity | Self::Both)
    }
}

/// The clamped popularity setter (party, value) invoked by the call sites.
pub const SET_POPULARITY_RVA: Rva = Rva(0x123_3110);

/// Each site, how it is patched and the domain its call moves.
pub const MUTATIONS: [(MutationSite, Patch, Domain); 9] = [
    // Capital setting (country, state, preserve): copy the stack pointer to
    // R11, then spill RBX through it to RCX's home slot.
    (
        MutationSite {
            name: "set_capital",
            rva: Rva(0x75_8f50),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    // Politics setting (status, party, allowed, frequency, date): copy the
    // stack pointer to RAX, then spill RBX through it to RCX's home slot.
    (
        MutationSite {
            name: "set_politics",
            rva: Rva(0xbf_b120),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    // Forced ruling-party selection (status, ideology group): save RBP and
    // reserve 0x30 stack bytes.
    (
        MutationSite {
            name: "force_ruling_party",
            rva: Rva(0xc0_0af0),
        },
        Patch::Entry,
        Domain::Both,
    ),
    // Ruling-party selection (status, group): spill RBX to RDX's home slot.
    (
        MutationSite {
            name: "set_ruling_party",
            rva: Rva(0xc0_1a30),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    // Idea replacement (status, removed, added, flags): copy the stack
    // pointer to RAX, then spill RBX through it to RCX's home slot.
    (
        MutationSite {
            name: "set_ideas",
            rva: Rva(0xc0_23f0),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    // Popularity addition (party, amount): spill RBX to RDX's home slot.
    (
        MutationSite {
            name: "add_popularity",
            rva: Rva(0x123_0770),
        },
        Patch::Entry,
        Domain::Popularity,
    ),
    // Redistributing popularity assignment (party, popularity) has the same
    // prologue as popularity addition; the site identity distinguishes them.
    (
        MutationSite {
            name: "set_popularity_redistributing",
            rva: Rva(0x123_2fe0),
        },
        Patch::Entry,
        Domain::Popularity,
    ),
    // Clamped popularity call in the political-party effect.
    (
        MutationSite {
            name: "set_political_party_popularity",
            rva: Rva(0x37_c50d),
        },
        Patch::Call,
        Domain::Popularity,
    ),
    // Clamped popularity call inside the popularities effect's party loop.
    (
        MutationSite {
            name: "set_popularities",
            rva: Rva(0x37_c847),
        },
        Patch::Call,
        Domain::Popularity,
    ),
];

/// Writers patched by another family, which forwards their calls by name
/// (`politics_sites::RESET_COUNTRY` and the `PoliticsDaily` scope).
/// Appearance reset moves both domains; daily update moves popularity and
/// structure only when last election changes across the call.
pub const FORWARDED: [&str; 2] = ["reset_country", "politics_daily"];

#[cfg(test)]
mod tests {
    use super::*;

    /// Every site is distinct, and no politics capture site patches the same address.
    #[test]
    fn sites_are_distinct_and_patch_no_capture_site() {
        for (index, (site, _, _)) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[..index]
                    .iter()
                    .all(|(other, _, _)| other.rva != site.rva && other.name != site.name)
            );
            let capture = [
                crate::politics_sites::ADD_POLITICAL_POWER,
                crate::politics_sites::ADD_POLITICAL_POWER_NO_DEFICIT,
                crate::politics_sites::SET_POLITICAL_POWER,
                crate::politics_sites::CLEAR_POLITICS,
                crate::politics_sites::RESET_COUNTRY,
                crate::politics_sites::SET_RULING_PARTY,
                crate::politics_sites::FORCE_RULING_PARTY,
                crate::politics_sites::SETUP_POLITICS,
                crate::politics_sites::FALLBACK_RULING_PARTY,
            ];
            for other in capture.iter().chain(crate::politics_sites::SCOPES.iter()) {
                // Interior commits may share a function, never the patched bytes.
                assert!(
                    other.rva.after(5) <= site.rva || site.rva.after(5) <= other.rva,
                    "{} overlaps {}",
                    site.name,
                    other.name
                );
            }
        }
    }

    /// Every popularity writer moves popularity, every structure writer moves
    /// structure, and only forced ruling-party selection writes both.
    #[test]
    fn each_site_names_the_domains_it_writes() {
        for (site, patch, domain) in &MUTATIONS {
            let popularity = *patch == Patch::Call
                || matches!(
                    site.name,
                    "add_popularity" | "set_popularity_redistributing" | "force_ruling_party"
                );
            let structure = !matches!(
                site.name,
                "add_popularity"
                    | "set_popularity_redistributing"
                    | "set_political_party_popularity"
                    | "set_popularities"
            );
            assert_eq!(
                (domain.structure(), domain.popularity()),
                (structure, popularity),
                "{}",
                site.name
            );
        }
    }
}
