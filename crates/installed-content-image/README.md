# installed-content-image

Native bitmap decoding and image representations, independent of content layers and catalogues. Callers resolve and read assets through [installed-content](../installed-content/README.md), then pass the requested name and bytes to the decoder.

[lib.rs](src/lib.rs) exposes RGBA and palette-indexed images, with BMP, DDS, TGA and RGBA conversion modules. DDS textures retain authored mip chains, cube faces and compressed payloads.

[native](src/native/mod.rs) implements the game's DDS, named-TGA and FreeImage decoding chain, including BMP, ICO, JPEG, PNG and TARGA plugins. Callers supply texture settings through `Decode`; `Failure` distinguishes undecodable bytes, undefined output, device-dependent output and a caller's texel budget.

The `test-support` feature exposes PNG and DX10 DDS fixture builders through `native::tests_support`. Decoder tests and their image fixtures live beside the implementation; GUI resource propagation tests belong to installed-content.

Read the [project rules](../../AGENTS.md) before changing a decoder. Modified Rust adaptations of IJG libjpeg 9a and libpng 1.6.16 retain their source notices; shared notices are in [LICENSES](../../LICENSES/). This software is based in part on the work of the Independent JPEG Group.
