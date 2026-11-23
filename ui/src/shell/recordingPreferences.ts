import { invoke } from '@tauri-apps/api/core';
import { invokeWrite } from './writes';
import { shareCampaignFlags } from '../dashboard/campaignFlags';

interface RecordingPreferences { flags: Record<string, string> }

export async function shareRecordingPreferences(): Promise<void> {
  await shareCampaignFlags(invoke<RecordingPreferences>('recording_preferences').then(preferences => preferences.flags),
    (campaignKey, tag) => invokeWrite<string | null>('set_campaign_flag', { campaignKey, tag }));
}
