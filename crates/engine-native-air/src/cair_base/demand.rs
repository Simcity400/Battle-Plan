use super::*;
use std::cell::Cell;

struct Counting<'a, M> {
    source: &'a M,
    spans: Cell<usize>,
    bytes: Cell<usize>,
    failure: Cell<Option<&'static str>>,
}

impl<M: Memory> Memory for Counting<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if !output.is_empty() {
            let spans = self.spans.get().checked_add(1);
            let bytes = self.bytes.get().checked_add(output.len());
            let (Some(spans), Some(bytes)) = (spans, bytes) else {
                self.failure.set(Some("carrier raw sizing extent overflow"));
                return false;
            };
            self.spans.set(spans);
            self.bytes.set(bytes);
        }
        self.source.copy(address, output)
    }
}

pub(super) fn raw_demand(
    memory: &impl Memory,
    image: usize,
    state: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    wings: usize,
    hint: &mut CarrierDemand,
) -> Result<(), &'static str> {
    let slots = (countries.arrays()[0].count as usize).saturating_sub(1);
    hint.raw_spans = slots
        .checked_mul(7)
        .and_then(|count| count.checked_add(2))
        .ok_or("carrier census sizing extent overflow")?;
    hint.raw_bytes = slots
        .checked_mul(40)
        .and_then(|count| count.checked_add(32))
        .ok_or("carrier census sizing extent overflow")?;
    let allocate = || {
        Ok::<_, std::io::Error>((
            engine_allocation::default_vec::<Row>(hint.bases)?,
            engine_allocation::default_vec::<RegistrationRow>(hint.registrations)?,
            engine_allocation::default_vec::<u64>(wings)?,
            engine_allocation::default_vec::<ResolvedLink>(hint.resolved)?,
            engine_allocation::default_vec::<CarrierBase>(hint.bases)?,
            engine_allocation::default_vec::<CarrierRegistration>(hint.registrations)?,
            engine_allocation::default_vec::<CarrierPool>(hint.pools)?,
            engine_allocation::default_vec::<CountryOccurrence>(hint.countries)?,
            engine_allocation::default_vec::<u8>(hint.name_bytes)?,
            engine_allocation::default_vec::<Option<CarrierIndexedCertificate>>(hint.indexed)?,
            engine_allocation::default_vec::<ResolvedLink>(hint.resolved)?,
            engine_allocation::default_vec::<CarrierCountryTerms>(hint.country_terms)?,
            engine_allocation::default_vec::<CarrierAceSlot>(hint.aces)?,
            engine_allocation::default_vec::<CarrierForecastSlot>(hint.forecasts)?,
        ))
    };
    let (
        mut base_rows,
        mut registrations,
        mut wing_rows,
        mut resolved,
        mut bases,
        mut carrier_registrations,
        mut pools,
        mut dense,
        mut names,
        mut indexed,
        mut carrier_resolved,
        mut country_terms,
        mut aces,
        mut forecasts,
    ) = allocate().map_err(|_| "carrier raw sizing allocation failed")?;
    let source = Counting {
        source: memory,
        spans: Cell::new(0),
        bytes: Cell::new(0),
        failure: Cell::new(None),
    };
    let result = (|| {
        let base_count = match walk(memory, image, state, &mut base_rows) {
            Outcome::Rows { len } => len,
            Outcome::Unavailable { reason } => return Err(reason),
            Outcome::Truncated { .. } => return Err("carrier base sizing observation changed"),
        };
        let countries = ccountry::Reader::new(&source, image, state)?;
        let mut capture = CarrierCapture::new(
            &source,
            image,
            state,
            &countries,
            CarrierBuffers {
                bases: &mut bases,
                registrations: &mut carrier_registrations,
                pools: &mut pools,
                countries: &mut dense,
                names: &mut names,
                indexed: &mut indexed,
                resolved: &mut carrier_resolved,
                country_terms: &mut country_terms,
                aces: &mut aces,
                forecasts: &mut forecasts,
            },
        )
        .with_transport_planes(Ok(()));
        let walked = walk_registrations_with_carrier(
            memory,
            image,
            state,
            RegistrationBuffers {
                registrations: &mut registrations,
                wings: &mut wing_rows,
                resolved: &mut resolved,
            },
            &base_rows[..base_count],
            &mut capture,
        );
        match walked.outcome {
            Outcome::Rows { .. } => Ok(()),
            Outcome::Unavailable { reason } => Err(reason),
            Outcome::Truncated { .. } => Err("carrier registration sizing observation changed"),
        }
    })();
    hint.raw_spans = hint
        .raw_spans
        .checked_add(source.spans.get())
        .ok_or("carrier raw sizing extent overflow")?;
    hint.raw_bytes = hint
        .raw_bytes
        .checked_add(source.bytes.get())
        .ok_or("carrier raw sizing extent overflow")?;
    source.failure.get().map_or(result, Err)
}
