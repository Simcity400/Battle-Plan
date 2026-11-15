//! The development harness and tooling around the map; not shipped. The desktop app hosts the map
//! itself (`viewer::host`).

/// The system heap slows sharply once a document holds gigabytes of scans; every product and
/// render allocation runs faster on this, as in the desktop app.
#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

fn main() -> std::process::ExitCode {
    viewer::cli()
}
