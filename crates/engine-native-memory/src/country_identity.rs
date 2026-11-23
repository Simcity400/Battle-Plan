use crate::cpdx_array::Array;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Country {
    pub pointer: usize,
    pub canonical_slot: u32,
    pub id: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ReservedCountryObservation {
    pointer: Option<usize>,
    result: Result<Country, &'static str>,
}

impl std::fmt::Debug for ReservedCountryObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReservedCountryObservation")
            .field("result", &self.result)
            .finish_non_exhaustive()
    }
}

impl ReservedCountryObservation {
    pub fn new(pointer: Option<usize>, result: Result<Country, &'static str>) -> Self {
        Self { pointer, result }
    }

    pub fn result(self) -> Result<Country, &'static str> {
        self.result
    }

    pub fn with_result(self, result: Result<Country, &'static str>) -> Self {
        Self { result, ..self }
    }

    pub fn from_result(result: Result<Country, &'static str>) -> Self {
        Self {
            pointer: result.ok().map(|country| country.pointer),
            result,
        }
    }

    pub fn physical_alias(self, pointer: usize) -> Option<Result<(), &'static str>> {
        (pointer != 0 && self.pointer == Some(pointer)).then(|| self.result.map(|_| ()))
    }
}

#[derive(Clone, Copy)]
pub struct AuthenticatedRoster<'a> {
    slots: &'a [Option<Country>],
    arrays: [Array; 2],
}

impl<'a> AuthenticatedRoster<'a> {
    pub fn new(slots: &'a [Option<Country>], arrays: [Array; 2]) -> Self {
        Self { slots, arrays }
    }

    pub fn slots(&self) -> &'a [Option<Country>] {
        self.slots
    }

    pub fn arrays(&self) -> [Array; 2] {
        self.arrays
    }
}
