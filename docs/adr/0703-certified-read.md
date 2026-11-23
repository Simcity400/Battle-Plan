# Publish reusable reads only after their writer certificate closes

A reusable observation needs a writer certificate that includes the input identity. The certificate provider owns that guarantee. A matching current certificate allows the cached value to carry because it excludes both relevant writes and identity changes.

A fresh read starts with a pending certificate. The reader first revokes the previous proof, then admits a replacement proof only when the read succeeds and the certificate remains unchanged. Abandoned reads, failed partial writes, or missing certificates require the next caller to read again. This lets the worker reuse its storage without mistaking the remaining bytes for a certified observation.

[Certified reads](../../crates/engine-native-memory/src/certified_read.rs) · [Equipment category certificates](../../crates/engine-native-parts/src/cequipment_type/category_cache.rs)
