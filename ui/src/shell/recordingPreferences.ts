import { invoke } from '@tauri-apps/api/core';
import { shareCampaignFlags } from '../dashboard/campaignFlags';

interface RecordingPreferences { flags: Record<string, string> }

/** Campaign flags live in the recorder's shared settings; those saved on this device move there once. */
export async function shareRecordingPreferences(): Promise<void> {
  // Choices made while the shared flags are read are newer than them.
  await shareCampaignFlags(invoke<RecordingPreferences>('recording_preferences').then(preferences => preferences.flags),
    (campaignKey, tag) => invoke('set_campaign_flag', { campaignKey, tag }));
}
