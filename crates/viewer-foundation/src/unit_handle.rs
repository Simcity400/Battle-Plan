
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unit {
    Division(u64),
    RailwayGun(u64),
    TaskForce(u64),
}
