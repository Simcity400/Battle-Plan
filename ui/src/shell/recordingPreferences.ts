import { invoke } from '@tauri-apps/api/core';
import { shareCampaignFlags } from '../dashboard/campaignFlags';

interface RecordingPreferences { flags: Record<string, string> }

/** Campaign flags live in the recorder's shared settings; those saved on this device move there once. */
export async function shareRecordingPreferences(): Promise<void> {
  const preferences = await invoke<RecordingPreferences>('recording_preferences');
  await shareCampaignFlags(preferences.flags, (campaignKey, tag) => invoke('set_campaign_flag', { campaignKey, tag }));
}
