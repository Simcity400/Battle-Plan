//! Prepared regional weather conditions and independently witnessed raw modifiers.

#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod products {
    pub mod regional_weather;

    dashboard_base::reply_warnings!(
        regional_weather::RegionalWeatherReply,
        regional_weather::RegionalWeatherProvincesReply,
    );
}
