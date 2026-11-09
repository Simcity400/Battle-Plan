//! Membership writers include inline assignment, deletion-only removal and root/parent repair.
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy)]
pub enum Receiver {
    Group,
    Order,
    /// Append CALL whose first argument is the order's embedded assigned-member vector.
    AssignedMembersCall,
    Every,
}

pub const MUTATIONS: [(MutationSite, Receiver); 19] = [
    (
        site("invalidate_member_orders", Rva(0xc47630)),
        Receiver::Group,
    ),
    (site("add_primary_order", Rva(0xc40500)), Receiver::Group),
    (site("remove_primary_order", Rva(0xc49f20)), Receiver::Group),
    (
        site("remove_fallback_order", Rva(0xc41f20)),
        Receiver::Group,
    ),
    (
        site("create_fallback_order", Rva(0xc416c0)),
        Receiver::Group,
    ),
    (site("repair_order_roots", Rva(0xc47340)), Receiver::Group),
    (site("set_parent_group", Rva(0xc4b310)), Receiver::Group),
    (site("dissolve_army_group", Rva(0xc497b0)), Receiver::Group),
    (
        site("delete_order_connections", Rva(0x18d14d0)),
        Receiver::Group,
    ),
    (site("add_assigned_member", Rva(0x10b0c10)), Receiver::Order),
    (
        site("remove_assigned_member", Rva(0x10cbeb0)),
        Receiver::Order,
    ),
    (
        site("remove_member_keep_branch", Rva(0x10cc020)),
        Receiver::Order,
    ),
    (
        site("remove_member_subtree", Rva(0x10c1e20)),
        Receiver::Order,
    ),
    (
        site("append_assigned_member", Rva(0x10c3d0a)),
        Receiver::AssignedMembersCall,
    ),
    (
        site("clear_assigned_members", Rva(0x10cb710)),
        Receiver::Order,
    ),
    (site("destroy_order", Rva(0x10ae220)), Receiver::Order),
    (site("load_order_members", Rva(0x10c40a0)), Receiver::Every),
    (site("load_group_orders", Rva(0xc48ae0)), Receiver::Every),
    (
        site("remove_deleted_group_member", Rva(0xc48410)),
        Receiver::Group,
    ),
];

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
}
