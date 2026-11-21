# 0505. Tie recorder controls to the attached session

An attached session, including one at the game's menu, keeps the capture mode selected when it attached. The recording represents one mode. Changing preferences affects a future session. The panel uses shared WebView appearance settings. It reads status after registering its listener so a status arriving during the first read cannot be lost. It also reads after a failed save because a recording can already have been written.

The panel names a recording by its existing or planned file name when known. It reports the current spool and the earlier saved recording it continues as separate sizes. Unknown progress is indeterminate. Loads that were not recorded have their own status. Background compression can rewrite a recording to High while the configured mode is Low. The action is unavailable for a recording already at that level or being compressed to it and can be retried after a retained original.

The Stop prompt belongs to the recording it opened against and closes when it stops or another replaces it. It advances its duration label independently because a paused game sends no new hour snapshots. Choosing Discard changes the controls and focuses Cancel, so a second click cannot accept the destructive action. Discard deletes only the unsaved session when an older recording exists. Opening this confirmation from Quit preserves the Quit intent until the prompt is answered or cancelled. The compact tray prompt closes when it is no longer needed.

[Recorder panel](../../ui/src/recorder/RecorderPanel.tsx), [Status projection](../../ui/src/recorder/status.ts), [Tray API](../../ui/src/recorder/api.ts)
