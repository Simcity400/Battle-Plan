# 0505. Keep update offers shared and restart requests scoped to a download

The recorder and desktop app share a build, installer, channel and release version. An installer launch closes both apps, so a reply to the launch request means it did not start. The UI checks whether the recorder can be replaced, and it asks for confirmation after the download. A notice or restart question belongs to the downloaded version and goes away if the offer changes. The current offer chooses the button action even when an earlier action failed. The update listener starts before the first snapshot request so a publish during that request is not lost.

[Update API](../../ui/src/update/api.ts), [Update control](../../ui/src/update/UpdateControl.tsx), [Update settings](../../ui/src/update/UpdateSettings.tsx), [Release channels](0001-release-channels.md)
