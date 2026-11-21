# Keep texture sampling consistent with the resource loader

Texture extents, compositing and colour affect sprites and map geometry. Try DDS magic first and the direct Targa reader when the logical name ends in `.tga`. For other textures, use content to select a decoder. Unsupported or undrawable formats produce a named refusal. Budget checks count the padded texels before decoding.

For DDS, preserve the classic loader's block-rounded extents and the mip prefix present before a file ends or an axis reaches one. Cubemaps store each complete chain in face order, and a file that cannot establish the whole cube is refused. Nonblock formats keep the selected layout, including unexpanded 16-bit channels. Turn X1R5G5B5's spare bit into opaque A1R5G5B5 before padding, modelling a device that supports that format. BC7 via a DX10 header is a separate supported format. It uses the logical extent and complete authored mip chain. Both zero extents produce at most two empty levels because further empty levels cannot change whether the device can create the texture. The UI's CPU expansion honours the decoder's admitted extent and samples 16-bit channels as the device does.

The client's power-of-two setting places each old level at the top-left of a new zeroed level, preserving the number of levels and copying compressed formats in whole blocks. Consumers use the padded extent for sizes and UVs. A zeroed texel samples as opaque black in a format without alpha and as transparent black where the stored zero is transparent. A padded level with zero extent cannot establish the texels the device would upload and is refused.

The direct Targa reader preserves the stored alpha byte in 32-bit images even when the descriptor declares no alpha bits. It treats the spare bit in 16-bit X1R5G5B5 pixels as opaque. The fallback also accepts Targa files identified by their footer or logical name.

The IJG 9a JPEG reader uses the fast eight-by-eight inverse DCT and disables fancy upsampling. Adobe CMYK samples are converted to opaque RGB. Arithmetic, progressive and scaled JPEG variants stay on the same JPEG reader. [IJG licence notice](../../LICENSES/IJG-libjpeg-README.txt)

The Rust `image` library decodes BMP, ICO, PNG and fallback Targa into RGBA. Pixel rounding, PNG colour and gamma handling, and the treatment of malformed files can differ from the game's loader. An undecodable file produces a named refusal.

[Native texture dispatch](../../crates/content-reader-image/src/native/mod.rs) · [DDS](../../crates/content-reader-image/src/native/load_dds.rs) · [JPEG](../../crates/content-reader-image/src/native/libjpeg.rs) · [Fallback decoders](../../crates/content-reader-image/src/native/standard.rs)
