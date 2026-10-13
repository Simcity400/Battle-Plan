import { invoke } from '@tauri-apps/api/core';
import { shareCampaignFlags } from '../dashboard/campaignFlags';
import { shareRecordingNames } from '../dashboard/recordingNames';

interface RecordingPreferences { names: Record<string, string>; flags: Record<string, string> }

/** In the desktop app, recording names and campaign flags live in the recorder's
 * shared settings so Battle Plan Recorder shows the same ones. Names and flags saved on
 * this device move there the first time. */
export async function shareRecordingPreferences(): Promise<void> {
  const preferences = await invoke<RecordingPreferences>('recording_preferences');
  await shareRecordingNames(preferences.names, changes => invoke('set_recording_names', { changes }));
  await shareCampaignFlags(preferences.flags, (campaignKey, tag) => invoke('set_campaign_flag', { campaignKey, tag }));
}
