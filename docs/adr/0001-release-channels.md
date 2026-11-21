# 0001. Build one installer per commit and promote it to a channel

The recorder and desktop app ship together at the same version, in one installer. A release is built once and promoted, so the installer that ships is the one that was tested. Both happen in GitHub Actions on GitHub-hosted runners, which are free for this public repository, following T3 Code's release workflow. Boat is for development, never a fallback. The steps are commands of `tools/release.py`, which the workflows call.

Main's CI builds a nightly candidate for every commit of main on a Windows runner (`release-candidate.yml`), using the release profile. It stamps the version, compiles both apps with this repository's releases as their feed address, and packages the NSIS installer. It then runs the leak gate on the installer and its UI bundle, and checks the installed layout. The map-drawing smoke needs a game installation, so it runs only on a machine that has one. The installer and a manifest of its commit, version, hash and the apps' updater key are kept for 30 days as an artifact named for the channel and commit, and for stable also the version. Rerunning the candidate workflow for a commit replaces an expired one.

The release workflow (`release.yml`) has three channels:
- **Nightly** is automatic. Every 30 minutes it publishes main's head when main has changed since the commit the nightly feed offers and main's CI is green on it: the candidate run succeeded and every other check on the commit settled green. Main's CI is every workflow the push started; scheduled and dispatched runs, the release's own among them, don't count. It does nothing otherwise, or while the repository variable `PAUSE_NIGHTLY` is true. It promotes the candidate main's CI built, without compiling.
- **Stable** is manual, a `workflow_dispatch`. By default it takes the commit the nightly feed offers and the version that nightly previews. Its candidate needs its own fat-LTO build, because the version is compiled in. A dispatch with `prepare` builds it ahead, so the release itself only promotes; without one, the release builds it first. After the release, a pull request sets main's workspace version to it and is squash-merged, since main takes changes only through pull requests. A merge by the workflow's token starts no workflow, so finalize then dispatches the candidate build for the new head. An older release never sets the version back.
- **Preview** is a test train: built on dispatch, published as a draft with no feed, never offered to installed apps.

Neither public channel installs itself. The apps show an update badge, then the user clicks to download and clicks to install.

The workspace version in Cargo.toml is the last stable release. A nightly or preview is the next patch with `-<channel>.<yyyymmdd>.<n>`: the UTC date of its build and the commit's position on main's first-parent line. Public channels take only commits on that line, so each position names one commit, and candidates built in main's order sort in that order. A stable candidate is that next patch, or a version named for a larger step. The version and the feed address are compiled into both apps, which compare the version with the feed's and record it in recordings, and the installer carries the version in its own metadata. Changing the version after the build means compiling again, and reading it at run time would let one binary claim several versions. So publishing never changes it, and a candidate belongs to one channel.

Releases of a channel run one at a time, queued, while dry runs run apart. A candidate is taken only from an artifact this repository's main built: the candidate workflow on a push or a dispatch, or the release workflow's build job. Its manifest must name the commit and version the release resolved. A commit whose workflows differ from main's is refused before building or signing, because GitHub refuses the workflow's token a release of it. Publishing takes the candidate and, in order:
1. It runs a preflight that fails with a named reason for each of these: the updater key is missing, is not one line of Tauri's base64 minisign secret key, has the wrong password, or is not the key the apps carry; some Azure settings are set but not all; no private terms are set; the repository is private and the channel is public.
2. It checks the installer's hash against its manifest.
3. It refuses:
   - a commit not on main's first-parent line, or one that does not contain the feed's commit;
   - a version not newer than the feed's or the channel's newest release;
   - a tag already on another commit.
4. It signs the installer with Authenticode when the Azure settings are set, then with the updater key, bound to its version. It verifies both and checks the signed installer's layout again.
5. It writes notes from the titles of pull requests merged since the channel's previous release.
6. It publishes the installer and its signature, then the feed, which it reads again just before replacing it. A client that sees a new feed can therefore download its target.
7. It checks the published assets against their hashes, and that the feed offers the channel's newest release.

Authenticode signs only the installer, because signing the binaries inside it would mean packaging again at release time. The certificate data it appends goes through the leak gate.

Each channel's feed is `latest.json` on a release named after the channel, kept a prerelease and never the latest, because the apps read `<releases>/download/<channel>/latest.json`. Each release's text records its source commit and the signed installer's hash. A rerun after the feed was published only checks it, so finalize still runs. A release whose feed upload failed is taken up again on a rerun, from its published installer, only while it is the channel's newest release and only if that installer is the one its text records. A dry run, `draft`, publishes everything to a new draft release and leaves the channel's feed alone.

The updater key and the Azure signing settings are secrets of the `release` environment, and of `release-stable`, which requires a reviewer. `LEAK_GATE_TERMS` is a repository secret. Release assets have no storage quota.

Both apps carry the Tauri updater verification key. Tauri decodes the key file as stored, so Git preserves its bytes and the file has no trailing newline. The installer smoke opens the app from an extracted install layout and reads a copied recording without installing software or writing user data.

Candidate builds map the runner's home, Cargo, rustup, checkout and target paths to neutral roots, so the binaries name nothing of the machine that built them. The leak gate checks the installer and the files inside it, the UI bundle, and the release notes and publication metadata. Installer asset names omit spaces, so their published names match feed URLs. The machine-path check allows specific checkout paths in the scripts that configure this machine. The leak gate allows authors' contact addresses in shipped licence notices. Private terms and signing-key material have no path exemptions.

[Release candidate](../../.github/workflows/release-candidate.yml) · [Release](../../.github/workflows/release.yml) · [Release steps](../../tools/release.py) · [Release smoke](../../tools/release_smoke.py) · [Leak gate](../../tools/leak-gate.py)
