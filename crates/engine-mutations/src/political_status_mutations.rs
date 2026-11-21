
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Patch {
    Entry,
    Call,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Structure,
    Popularity,
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

pub const SET_POPULARITY_RVA: Rva = Rva(0x123_3110);

pub const MUTATIONS: [(MutationSite, Patch, Domain); 9] = [
    (
        MutationSite {
            name: "set_capital",
            rva: Rva(0x75_8f50),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "set_politics",
            rva: Rva(0xbf_b120),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "force_ruling_party",
            rva: Rva(0xc0_0af0),
        },
        Patch::Entry,
        Domain::Both,
    ),
    (
        MutationSite {
            name: "set_ruling_party",
            rva: Rva(0xc0_1a30),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "set_ideas",
            rva: Rva(0xc0_23f0),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "add_popularity",
            rva: Rva(0x123_0770),
        },
        Patch::Entry,
        Domain::Popularity,
    ),
    (
        MutationSite {
            name: "set_popularity_redistributing",
            rva: Rva(0x123_2fe0),
        },
        Patch::Entry,
        Domain::Popularity,
    ),
    (
        MutationSite {
            name: "set_political_party_popularity",
            rva: Rva(0x37_c50d),
        },
        Patch::Call,
        Domain::Popularity,
    ),
    (
        MutationSite {
            name: "set_popularities",
            rva: Rva(0x37_c847),
        },
        Patch::Call,
        Domain::Popularity,
    ),
];

pub const FORWARDED: [&str; 2] = ["reset_country", "politics_daily"];

#[cfg(test)]
mod tests {
    use super::*;

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
                assert!(
                    other.rva.after(5) <= site.rva || site.rva.after(5) <= other.rva,
                    "{} overlaps {}",
                    site.name,
                    other.name
                );
            }
        }
    }

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
