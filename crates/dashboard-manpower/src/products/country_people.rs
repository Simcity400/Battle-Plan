//! A country's manpower view at an hour (`country-people`): its manpower operands from the
//! `manpower` family its native walk prepares ([`facts`], [`emit`]), read at the hour
//! ([`query`]) and made into the view by the game's rules in
//! `dashboard_base::products::country_people`, with its election schedule and the governments in
//! exile it is or hosts from `politics_state`.

pub mod emit;
pub mod facts;
pub mod query;
pub mod served;
#[cfg(test)]
mod tests;

pub use dashboard_base::products::country_people::*;
use dashboard_base::products::{Product, validate_tag};
use ts_rs::TS;

/// The family's name in the registry and its native binding.
pub const NAME: &str = "manpower";

pub struct CountryPeople;

impl Product for CountryPeople {
    const KEY: &'static str = "country-people";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = PeopleParams;
    type Reply = PeopleReply;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);

    fn types() -> Vec<String> {
        vec![
            PeopleParams::decl(),
            PeopleFact::decl(),
            PeopleState::decl(),
            PeopleExile::decl(),
            PeopleReply::decl(),
        ]
    }

    fn validate(p: &PeopleParams) -> Result<(), &'static str> {
        validate_tag(&p.tag)?;
        p.branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid people branch")?;
        Ok(())
    }
}
