//! Army, navy and air experience prepared on the segment driver: each hour's countries and
//! their joined banked and gain rows ([`facts`]), the experience scan stepped over them and
//! each read it publishes kept as runs with their holder evidence on the family's series
//! publication ([`emit`]), and the series family `country-experience` answered from it
//! ([`query`]). The scan, and the row product reading a whole branch, are
//! [`dashboard_base::products::country_experience`]'s.

pub mod emit;
pub mod facts;
pub mod query;

#[cfg(test)]
mod tests;
