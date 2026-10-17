# dashboard-subjects

The subjects and autonomy products: who was whose subject over a branch, and each subject's autonomy level, overlord, score and the scores at which it moves. Behaviour is documented beside the code in [products/subjects.rs](src/products/subjects.rs) and [products/autonomy.rs](src/products/autonomy.rs) and in the widget guides ([subjects](../../ui/src/widgets/subjects/README.md), [autonomy](../../ui/src/widgets/autonomy/README.md)).

This is a product crate above [dashboard-base](../dashboard-base/README.md), laid
out like [dashboard-politics-ledger](../dashboard-politics-ledger/README.md): the
modules keep their dashboard paths, `products::subjects`, `products::autonomy`, and the code names what it uses
at the crates that own it ([dashboard-base](../dashboard-base/README.md),
[dashboard-text](../dashboard-text/README.md) and [dashboard-core](../dashboard-core/README.md)).
The [dashboard](../dashboard/README.md) re-exports them at the old
`dashboard::` paths, registers them, and prepares and queries them in its pipeline.
Autonomy's `TABLES` declares the observer's inputs for the shared walk and direct reads.

`cargo test -p dashboard-subjects` builds and runs only these tests. The `test-support`
feature compiles the test oracles and instrumentation for the dashboard's tests.
