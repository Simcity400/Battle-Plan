//! Names of provinces and states at hours (`place-names`): each one's name inputs from the
//! `place_names` family its native walk prepares ([`facts`], [`emit`]), read at each requested
//! hour ([`query`]) and named by the ladder `dashboard_base::products::place_names` keeps,
//! which the frame reader names by too.

pub mod emit;
pub mod facts;
pub mod query;
#[cfg(test)]
mod tests;

pub use dashboard_base::products::place_names::*;
use dashboard_base::products::Product;

/// The family's name in the registry and its native binding.
pub const NAME: &str = "place_names";

pub struct PlaceNames;

impl Product for PlaceNames {
    const KEY: &'static str = "place-names";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = PlaceNamesParams;
    type Reply = PlaceNamesReply;

    fn types() -> Vec<String> {
        types()
    }

    fn validate(params: &PlaceNamesParams) -> Result<(), &'static str> {
        validate(params)
    }
}
