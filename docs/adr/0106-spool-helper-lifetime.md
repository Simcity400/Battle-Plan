# Join borrowed spool work before returning or unwinding

Persistent conversion helpers start before hook publication and park between hours. The spool uses eligible CPUs beyond its own, up to the number of available packs, and limits parallel comparison and encoding to two helpers. Frame priority manages promotion without EcoQoS or CPU-set confinement. Callers fix output positions so work produces the same result as serial processing.

`SpoolHelper::join` sends a borrowed `Sync` job into a helper with the borrow erased and runs its own share locally. The helper can dereference the pointer only while it owns the running job. The caller waits through publication and completion, including when its own share unwinds. The `Finish` guard performs the wait on unwind, and the normal path propagates a helper panic only after the helper releases the job. A helper that has ended can no longer hold the pointer. Nested joins propagate the same lifetime rule to every helper involved.

[Helper implementation](../../crates/collector-workers/src/spool_helper.rs)
