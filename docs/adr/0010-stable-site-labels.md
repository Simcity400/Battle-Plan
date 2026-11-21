# 0010. Keep generated site names stable through source cleanup

The locator generator derives some symbolic site names from the closest `name` field within a fixed number of source lines. Removing comments can move a neighboring label into that window even when the Rust tokens are identical. Event provenance and politics site data retain the blank lines required to preserve those names. The audit compared both the set of site keys and their selected names with the baseline.

`Image::module` borrows a loaded PE32+ module image. Its caller must pass the base of a module that stays loaded for the lifetime of the program. The caller-facing Rustdoc retains this lifetime contract.

[Site generator](../../tools/sites.py) · [Image handle](../../crates/engine-sites/src/image.rs)
