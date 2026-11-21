# 0001. Build one installer and publish channel feeds separately

The recorder and desktop app ship together at the same version. A preview uses the same signed installer build as the release channels, with the update feed omitted. It stays a workflow artifact and is never offered to installed apps. Stable and nightly release feeds must be public because installed apps read them directly. Tag and scheduled triggers are disabled while the repository is private. Once public, nightly can be run manually on main even if its head has already shipped.

The Windows runner builds after any game run and within its slot of the shared preparation lock. The job timeout includes that wait. It retains the build cache within its size limit. Only stable uses the slower fat-LTO build. Release resolution and publication use GitHub-hosted runners and the workflow's own token.

Both apps carry the Tauri updater verification key. Tauri decodes the key file as stored, so Git preserves its bytes and the file has no trailing newline. The build verifies the updater signature before publishing. Authenticode is used when all Azure signing settings are available. The installer smoke test opens the app from an extracted install layout and reads a copied recording without installing software or writing user data.

Release builds mask the runner's home directory in GitHub logs. They map Rust paths to neutral roots and set the mappings only in the build process. The most specific mapping goes last because rustc uses the last match. Cargo's encoded flags keep each mapping together even if a path has spaces. The leak gate checks the installer and the files it contains, the embedded frontend, and the release notes and publication metadata. Installer asset names omit spaces so their published names match feed URLs.

The machine-path check allows specific checkout paths in the scripts that configure this machine. The leak gate allows authors' contact addresses in shipped licence notices. Private terms and signing-key material have no path exemptions.

The workflow serializes publishing within each channel and rejects versions older than the current feed or the latest stable release. Notes come from merged pull request titles, which require history even for a blob-filtered checkout. An installer and its signature are available before `latest.json` points to them, so a client that sees a new feed can download its target.

[Release workflow](../../.github/workflows/release.yml) · [Installer build](../../.github/workflows/release-build.yml)
