//! Recorded unit handles shared by counters, selected movement and world tracks.

/// Native unit classes have separate handle namespaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unit {
    Division(u64),
    RailwayGun(u64),
    TaskForce(u64),
}
