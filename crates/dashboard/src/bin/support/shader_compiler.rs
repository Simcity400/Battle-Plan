const MISSING: &str = "shader compiler not found: sprites, icons and portraits are refused";

pub(super) fn configure() -> &'static str {
    let source = match shader_pipeline::Compiler::discover() {
        Ok(compiler) => {
            // Both callers run before any worker threads start or LOCALAPPDATA changes.
            unsafe { std::env::set_var("BATTLE_PLAN_SHADER_COMPILER", compiler.executable()) };
            compiler.source()
        }
        Err(reason) => {
            eprintln!("shaderCompiler: {reason}");
            MISSING
        }
    };
    eprintln!("shaderCompiler: {source}");
    source
}
