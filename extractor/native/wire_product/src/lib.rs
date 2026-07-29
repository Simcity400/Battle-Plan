//! Production packaging boundary for the rebuilt replay wire format.
//!
//! `wire_format` owns canonical frame/schema/gap bytes. This crate owns the
//! extractor-side product around those bytes: authenticated symbol assignment,
//! branch and schema registries, domain blocks, gap artifacts, a streaming
//! H4CR reader, and a semantic reopen gate over the exact bytes to be packed.
//! The first promoted domain is d81. Adding another domain requires a new
//! adapter and verifier arm; unknown domains are never passed through opaquely.

mod build;
mod catalog;
mod codec;
mod model;
mod retained_stream;
mod verify;

pub use build::{
    build_d81_bundle, build_d81_bundle_from_file, build_d81_bundle_with_leading_gaps,
    inspect_d81_session,
};
pub use catalog::{AuthenticatedProvinceCatalog, ProvinceBinding};
pub use model::{
    ArtifactRole, BranchParent, BranchSpec, BundleBuildConfig, BundleEntry,
    BundleEntryDescriptor,
    D81LeadingGap, D81LeadingGapKind, D81SessionInspection, H4crSelection, H4crSession,
    H4crSource, MaterializedBundle, MaterializedEntry, PreparedBundle, ProductError,
    VerifiedBranch, VerifiedBundle,
};
pub use verify::{
    verify_entries, verify_entry_source, verify_entry_source_against_h4cr,
    verify_prepared_bundle,
};

#[cfg(test)]
mod tests;
