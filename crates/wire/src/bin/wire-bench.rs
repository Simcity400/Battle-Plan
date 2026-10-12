//! Full validating-reader cost over Rust-written, bounded synthetic slice units.
use std::{hint::black_box, time::Instant};
use wire::{fixtures, model::ReadContext, reader, shapes};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations: usize = std::env::var("H4R_BENCH_ITERATIONS")
        .unwrap_or_else(|_| "80".into())
        .parse()?;
    if iterations == 0 {
        return Err("invalid-benchmark-iterations".into());
    }
    let registry = shapes::registry();
    let fixtures = fixtures::valid_fixtures()?;
    let mut measurements = Vec::new();
    for fixture in fixtures.iter().filter(|f| {
        matches!(
            f.name.as_str(),
            "5000-country-rank-window"
                | "viewport-chart-16384-runs"
                | "window-focus-16384-segments"
                | "window-produced-16384-exact-samples"
        )
    }) {
        let context = ReadContext {
            expected_bounds: fixture.expected_bounds,
            require_bounds_match: true,
        };
        for _ in 0..10 {
            black_box(reader::read(&fixture.bytes, &registry, context)?);
        }
        let mut costs = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = Instant::now();
            black_box(reader::read(black_box(&fixture.bytes), &registry, context)?);
            costs.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        costs.sort_by(f64::total_cmp);
        let p99 = iterations - iterations / 100 - 1;
        measurements.push(serde_json::json!({
            "name":fixture.name,"bytes":fixture.bytes.len(),"iterations":iterations,
            "medianMs":costs[iterations/2],"p99Ms":costs[p99],
        }));
    }
    if measurements.is_empty() {
        return Err("no-benchmark-fixtures".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"decode":measurements}))?
    );
    Ok(())
}
