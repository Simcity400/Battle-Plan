# Share update channels and coordinate installation between apps

Both apps follow a single saved channel and ship at the same version. Absent a user choice, the channel follows the running build. Builds without a release feed leave updates off. The updater uses Windows TLS and the system proxy, and checks the signed version in the minisign trusted comment as Tauri does. It accepts both whole-file and hashed signatures. A stalled download is timed by inactivity, and the progress control updates at whole-percentage increments.

The two apps publish independent state for a shared update. Actions wait out a running check on a blocking thread. A failed recheck preserves an offer already shown, alongside its failure reason. Before applying a downloaded offer, the action lock serializes a fresh channel read with checks. A change made in the other app drops the old channel's offer and schedules a check. A successful installer launch keeps the action busy while the app closes.

The Windows session and app role identify the update pipes. The installer-compatible protocol queries readiness with `Q` and requests close with `C`. A busy query carries a UTF-8 reason. The app waits for the caller to read its reply before closing, so the installer can distinguish which apps to reopen. Multiple pipe instances let several copies answer independently. Setup holds a guard checked at launch. During setup a separate PowerShell notice lets the process exit without keeping an installed file open.

The desktop and recorder use distinct restart flags. The recorder's flag does not start with `/R`, which the installer also uses as a prefix for the desktop restart. `ShellExecuteW` reports launch success with a result greater than 32. The shared Tauri commands live in a module because the root of a library cannot export them in the form Tauri needs.

[Updater](../../crates/app-update/src/lib.rs) · [Control protocol](../../crates/app-update/src/control.rs) · [Feed verification](../../crates/app-update/src/feed.rs) · [Installer coordination](0006-installer-coordination.md)
