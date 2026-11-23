# Compose derived writers at the recorder

The derivation framework owns the writer contract, scheduling, recorded definitions and helpers shared across domains. The land, equipment, air, naval, economy, politics and world crates own their writers and their tests. The recorder's registry assembles those writers in the same order, preserving their input dependencies and journal behaviour. Replay accepts the writers from its caller so a domain change does not rebuild replay and the crates that read recordings. Journal encoding and recording fixtures supply the same registry at their entry points. Journal multiplication and change digests identify derived tables from the recording schema. The registry tests check table ownership and scheduling across the combined set.

[Framework](../../crates/recorder-derive-framework/src/framework.rs) ? [Writer registry](../../crates/recorder-derive/src/lib.rs) ? [Live writer](../../crates/replay/src/campaign/live.rs)
