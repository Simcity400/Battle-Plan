
#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

fn main() -> std::process::ExitCode {
    viewer::cli()
}
