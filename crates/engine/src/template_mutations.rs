//! The runtime writers of a division template's fallback name key
//! (template +0x40, contained data +0x28). This is the DivisionTemplates field the
//! hourly icon walk does not always read:
//! [`crate::creferenced_division_template::walk_icons`]
//! reads it only for an empty literal name. Observing writers tells the
//! collector which keys to reread; recording never calls them.
//!
//! Writer coverage:
//! - Editable-data assignment takes destination and source data and assigns
//!   literal name +0x08 and fallback key +0x28 through string byte assignment.
//!   Callers are the update command, three copy helpers and designer staging
//!   items. Full-data assignment also copies both names, from the creation
//!   wrapper and designer. The wrapper serves the create command, template
//!   addition and colonial-template effects, copy helpers and console army
//!   spawning.
//! - The data token reader parses both names; the primary reader forwards
//!   to it.
//! - The primary deleting destructor names a template before a new template
//!   can reuse its freed address.
//! - Template addition renames only the literal in place on a name conflict.
//!   The icon walk reads every literal hourly.

pub use crate::advisor_mutations::MutationSite;

/// How a site's RCX names the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// The contained data, `template+DATA`.
    Data,
    /// The template.
    Template,
}

/// The contained data's offset in the template
/// ([`crate::creferenced_division_template::COMPOSITION`]).
pub const DATA: usize = crate::creferenced_division_template::COMPOSITION;

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 4] = [
    // Editable-data assignment (dest, source): saving RSI and reserving 0x20
    // stack bytes occupy six relocated bytes.
    (
        MutationSite {
            name: "assign_editable_template_data",
            rva: 0xb8_f300,
            sentinel: [
                0x40, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x89, 0x6c, 0x24, 0x38, 0x48, 0x8b, 0xf1,
                0x48, 0x8b,
            ],
        },
        Receiver::Data,
    ),
    // Full-data assignment (dest, source): the RBX spill at stack +8 relocates.
    (
        MutationSite {
            name: "assign_template_data",
            rva: 0xb8_6d00,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x48,
            ],
        },
        Receiver::Data,
    ),
    // Data reader (data, reader, token): the RBX spill at stack +8 relocates.
    (
        MutationSite {
            name: "read_template_data",
            rva: 0xb9_37a0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Data,
    ),
    // Primary deleting destructor (template, flags): the RBX spill at stack +8 relocates.
    (
        MutationSite {
            name: "destroy_template",
            rva: 0xb8_7fc0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x9f,
                0x0f, 0xdb,
            ],
        },
        Receiver::Template,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The data receiver is the template's contained data, and the sites are
    /// distinct.
    #[test]
    fn data_sites_name_the_contained_data() {
        assert_eq!(DATA, 0x18);
        assert_eq!(crate::creferenced_division_template::NAME_KEY, DATA + 0x28);
        let mut rvas = MUTATIONS.map(|(site, _)| site.rva);
        rvas.sort_unstable();
        assert!(rvas.windows(2).all(|pair| pair[0] != pair[1]));
    }
}
