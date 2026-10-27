use product_codec::product;

product! {
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct CountryKey {
        pub branch: u64, pub native: u32, pub canonical_slot: u32,
        pub observed_hour: i32, pub continuity: String
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct EventKey {
        pub branch: u64, pub epoch: u64, pub sequence: u64, pub hour: i32,
        pub kind: u8
    }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct RefreshOccurrence { pub key: EventKey, pub declaration: u32 }
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct RefreshInterval {
        pub branch: u64, pub epoch: u64, pub from_hour: i32, pub to_hour: i32,
        pub first: u64, pub next: u64, pub occurrences: Vec<RefreshOccurrence>,
        pub status: crate::Availability
    }
}
