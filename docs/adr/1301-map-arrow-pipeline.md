# 1301. Share the authored arrow pipeline across map overlays

Battle plans, raid and trade arrows and map symbols share the authored effect and shadow input. They keep their own vertex declaration, binding layout and per-draw constants. Pipeline identity incorporates the authored technique, target format and draw mode so a pipeline from another combination cannot be reused. A visual mod that cannot render the replacement leaves the recorded effect drawable and is reported through the shader library.

Shadow constants update the light matrix, common lighting and time while preserving inherited camera terms. Late passes preserve the authored depth descriptor and adjust the stencil reference and masks. Shadow input binds only to the pixel stage. Per-draw payload validation distinguishes initialized constants from unused allocation padding so unread padding cannot become an apparent shader input.

[Arrow pipeline](../../crates/viewer-aircraft/src/arrow_shader.rs)
