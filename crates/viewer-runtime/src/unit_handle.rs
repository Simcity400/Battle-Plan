
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Unit {
    Division(u64),
    RailwayGun(u64),
    TaskForce(u64),
}
