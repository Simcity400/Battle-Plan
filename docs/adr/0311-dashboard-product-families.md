# 0311. Compile dashboard products by family

Dashboard product implementations and their registration functions live in family crates so each family compiles its own product readers and serializers. The dashboard service keeps the ordered registry and panel dispatch so request keys and the generated TypeScript contract keep their original order. The base crate supplies query machinery, and the world crate supplies shared country, window, region and equipment naming helpers. Each product owns its run cache through a distinct state type on the recording source. The dashboard open sequence warms the same run and branch readers used by requests. Forget clears the base query state and calls the owning families to clear their caches before a reload.

[Product registry](../../crates/dashboard/src/products/mod.rs) ? [Open and forget](../../crates/dashboard/src/answer.rs)
