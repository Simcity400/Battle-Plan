use std::path::Path;

#[derive(serde::Serialize)]
struct ContentAuthorityReport {
    schema: &'static str,
    recording: String,
    bundle_content_authority: crate::content_compatibility::BundleContentAuthority,
}

const CONTENT_AUTHORITY_REPORT_SCHEMA: &str = "hoi4.replay.content_authority_report.v1";

/// Opens one recording through the product policy, admits its recorded content
/// against this machine's install exactly as playback would, and writes the
/// `BundleContentAuthority` (ordered winner inputs + authority digest) as
/// JSON. Renders nothing; the RC capture predicate diffs these fields against
/// the offline-minted winner binding.
pub(crate) fn run(
    recording: &Path,
    out: &Path,
    logical_stream_id: Option<u64>,
    paths: &crate::install_paths::ProductPaths,
) -> crate::Result<()> {
    let source = match logical_stream_id {
        Some(id) => crate::recording_open::open_product_recording_segment(recording, Some(id)),
        None => crate::recording_open::open_product_recording(recording),
    }
    .map_err(|error| format!("open recording {}: {error}", recording.display()))?;
    let provider = crate::timeline::build_recording_provider(
        None,
        source,
        replay_core::TimelineConfig::default(),
        &recording.display().to_string(),
    )?
    .0;
    let (resolver, admission) =
        crate::content_compatibility::bind_provider_content(provider.as_ref(), paths)?;
    let manifest = provider
        .content_manifest()
        .ok_or("recording carries no portable used-content manifest")?;
    let authority = crate::content_compatibility::derive_bundle_content_authority(
        manifest,
        &resolver,
        &admission,
    )?;
    let report = ContentAuthorityReport {
        schema: CONTENT_AUTHORITY_REPORT_SCHEMA,
        recording: recording.display().to_string(),
        bundle_content_authority: authority,
    };
    let mut bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| format!("encode content-authority report: {error}"))?;
    bytes.push(b'\n');
    crate::recording_reach::write_report_create_new_or_equal(out, &bytes)?;
    println!(
        "content authority: {} ordered input(s), authority_sha256 {}",
        report.bundle_content_authority.ordered_inputs.len(),
        report.bundle_content_authority.authority_sha256
    );
    Ok(())
}
