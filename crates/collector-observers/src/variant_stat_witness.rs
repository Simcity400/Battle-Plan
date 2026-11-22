use std::sync::OnceLock;

type QuietProvider = fn() -> Result<(u64, u64), &'static str>;

static PROVIDER: OnceLock<QuietProvider> = OnceLock::new();

pub fn install(provider: QuietProvider) {
    let _ = PROVIDER.set(provider);
}

pub fn quiet() -> Result<(u64, u64), &'static str> {
    PROVIDER.get().map_or(
        Err("variant stat writer hooks disabled"),
        |provider| provider(),
    )
}

pub struct ObserverStatWitness;

impl engine_native::cequipment_variant::StatWitness for ObserverStatWitness {
    fn quiet(&self) -> Result<(u64, u64), &'static str> {
        quiet()
    }
}
