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
    ArtifactRole, BranchParent, BranchSpec, BundleBuildConfig, BundleEntry, BundleEntryDescriptor,
    D81LeadingGap, D81LeadingGapKind, D81SessionInspection, H4crSelection, H4crSession, H4crSource,
    MaterializedBundle, MaterializedEntry, PreparedBundle, ProductError, VerifiedBranch,
    VerifiedBundle,
};
pub use verify::{
    verify_entries, verify_entry_source, verify_entry_source_against_h4cr, verify_prepared_bundle,
};

#[cfg(test)]
mod tests;
