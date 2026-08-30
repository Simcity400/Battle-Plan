//! The PLS1 production-line subtype: its wire code, the legacy `kind` the same
//! row carries, and the name it is folded under.
//!
//! THE DEFECT (lane TV, row R18). Three crates that cannot see each other each
//! wrote this codebook down: the collector's own enum assigned `1..=4` and
//! spelled each name, `recorder_host`'s fold matched the four `(kind, subtype)`
//! pairs and spelled them again, and `wire_format`'s port restated the pairs a
//! third time. Exchanging `Military` and `RailwayGun` in the producer alone
//! keeps both valid kind-1 subtypes, so it compiles and every row is folded,
//! named and reported as the other.
//!
//! This crate is dependency-free, which is why all three may depend on it --
//! the owner's 2026-08-15 one-shared-decoder shape, as used for d64 and XUI09.
//! It is also an ATTESTED leaf, so the column is written out rather than
//! generated: `verify_world_fold_citations` refuses a `macro_rules!` here,
//! because the digest covers source text and the code that runs would be the
//! code a macro produced.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u32)]
pub enum ProductionLineSubtype {
    Military = 1,
    Naval = 2,
    ShipRefit = 3,
    RailwayGun = 4,
}

impl ProductionLineSubtype {
    pub const ALL: [ProductionLineSubtype; 4] = [
        ProductionLineSubtype::Military,
        ProductionLineSubtype::Naval,
        ProductionLineSubtype::ShipRefit,
        ProductionLineSubtype::RailwayGun,
    ];

    pub const CARDINALITY: u32 = ProductionLineSubtype::ALL.len() as u32;

    /// The PLS1 row's `subtype` word.
    pub const fn subtype(self) -> u32 {
        self as u32
    }

    /// The coarse production-line `kind` the same row must carry.
    pub const fn legacy_kind(self) -> u32 {
        match self {
            ProductionLineSubtype::Military | ProductionLineSubtype::RailwayGun => 1,
            ProductionLineSubtype::Naval | ProductionLineSubtype::ShipRefit => 2,
        }
    }

    pub const fn matches_legacy_kind(self, kind: u32) -> bool {
        self.legacy_kind() == kind
    }

    /// The name the fold publishes.
    pub const fn as_str(self) -> &'static str {
        match self {
            ProductionLineSubtype::Military => "military",
            ProductionLineSubtype::Naval => "naval",
            ProductionLineSubtype::ShipRefit => "ship_refit",
            ProductionLineSubtype::RailwayGun => "railway_gun",
        }
    }

    /// The subtype a PLS1 row names, read through `subtype`/`legacy_kind`
    /// rather than beside them. `None` is the incoherent pair the fold refuses
    /// the whole segment for.
    pub const fn from_wire(kind: u32, subtype: u32) -> Option<ProductionLineSubtype> {
        let mut i = 0;
        while i < ProductionLineSubtype::ALL.len() {
            let candidate = ProductionLineSubtype::ALL[i];
            if candidate.subtype() == subtype && candidate.legacy_kind() == kind {
                return Some(candidate);
            }
            i += 1;
        }
        None
    }

    /// The position in [`Self::ALL`]. A dense projection code for a consumer
    /// that needs one; NOT the wire value.
    pub const fn dense_code(self) -> u32 {
        let mut i = 0;
        while i < ProductionLineSubtype::ALL.len() {
            if ProductionLineSubtype::ALL[i] as u32 == self as u32 {
                return i as u32;
            }
            i += 1;
        }
        panic!("d11_grammar: a ProductionLineSubtype is not in ALL")
    }
}

const _: () = {
    let mut i = 0;
    while i < ProductionLineSubtype::ALL.len() {
        let a = ProductionLineSubtype::ALL[i];
        if a.subtype() == 0 {
            panic!("d11_grammar: 0 is the absent subtype and no member may claim it");
        }
        if a.legacy_kind() != 1 && a.legacy_kind() != 2 {
            panic!("d11_grammar: a subtype's legacy kind is not one the producer emits");
        }
        let mut j = i + 1;
        while j < ProductionLineSubtype::ALL.len() {
            let b = ProductionLineSubtype::ALL[j];
            if a.subtype() == b.subtype() {
                panic!("d11_grammar: two production-line subtypes share a wire code");
            }
            let (an, bn) = (a.as_str().as_bytes(), b.as_str().as_bytes());
            if an.len() == bn.len() {
                let mut k = 0;
                let mut same = true;
                while k < an.len() {
                    if an[k] != bn[k] {
                        same = false;
                    }
                    k += 1;
                }
                if same {
                    panic!("d11_grammar: two production-line subtypes are folded under one name");
                }
            }
            j += 1;
        }
        i += 1;
    }
};
