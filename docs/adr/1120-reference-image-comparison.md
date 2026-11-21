# Compare reference images over matched, unmasked pixels

The viewer's developer harness compares two images of the same hour and camera. The desktop hosts the map directly. Both use the same allocator because a document with gigabytes of scans makes allocation performance important.

The comparison reports gamma-encoded absolute RGB error, luminance SSIM and CIE76 distance in D65 Lab after decoding sRGB to linear light. Luminance is Rec. 601. The three SSIM levels begin at full resolution, using a separable Gaussian over complete windows. An excluded pixel disqualifies that window. A downsampled pixel is eligible only when all four contributing pixels were eligible, and an odd edge is dropped at each level. Missing sample sets leave their metrics unavailable. Luminance can stay stable across an obvious hue change, so the comparison retains colour distance as a separate metric.

Exclusion rectangles use inclusive origins and exclusive right and bottom edges. A mask image excludes pixels with all channels zero. The built-in interface masks are empirical, for the supported screenshot sizes and UI scale. The ultrawide mask bounds invariant luminance across several views and rounds out to leave a margin. Other window sizes and operating-system overlays need explicit exclusions. The remaining metrics cover the entire unmasked picture and the row-major region grid. The difference picture amplifies per-channel error and shades the excluded pixels separately.

When the viewer render needs upscaling to align it with the reference, bilinear samples align pixel centres, including at the clamped edges. Input PNGs use tightly packed top-left row-major RGB. RGBA inputs discard alpha before comparison.

[Reference comparison](../../crates/viewer/src/compare.rs) · [Comparison tests](../../crates/viewer/src/compare/tests.rs) · [Viewer harness](../../crates/viewer/src/bin/viewer.rs)
