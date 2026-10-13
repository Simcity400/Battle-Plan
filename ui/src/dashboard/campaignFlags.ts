import { useMemo, useSyncExternalStore } from 'react';
import { currentCampaignSettings, saveCampaignSettings, setCampaignFlag, useCampaignSettings } from './campaignSettings.ts';
import type { CampaignSettings } from './campaignSettings.ts';

/** The flag chosen for a campaign in the sidebar, as a recorded tag, by campaign key.
 * The desktop app keeps these in the recorder's shared settings, so Battle Plan Recorder
 * uses the same choice; the browser keeps them with this device's campaign settings. */
export type CampaignFlags = Record<string, string>;
type Save = (campaignKey: string, tag: string | null) => Promise<void>;
type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

let shared: { flags: CampaignFlags; save: Save } | null = null;
const listeners = new Set<() => void>();
const notify = () => { for (const listener of listeners) listener(); };
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const sharedFlags = () => shared?.flags ?? null;

export function settingsFlags(settings: CampaignSettings): CampaignFlags {
  return Object.fromEntries(Object.entries(settings.campaigns).flatMap(([key, entry]) => entry.flag ? [[key, entry.flag]] : []));
}

/** Use the recorder's shared settings for flags. A flag this device chose fills in a
 * campaign the shared settings lack, once; it then leaves this device's settings. */
export async function shareCampaignFlags(stored: CampaignFlags, save: Save, storage?: Storage): Promise<void> {
  const local = settingsFlags(currentCampaignSettings(storage));
  const flags = { ...stored };
  for (const [key, tag] of Object.entries(local)) {
    if (Object.hasOwn(flags, key)) continue;
    await save(key, tag);
    flags[key] = tag;
  }
  if (Object.keys(local).length) {
    let settings = currentCampaignSettings(storage);
    for (const key of Object.keys(local)) settings = setCampaignFlag(settings, key, null);
    saveCampaignSettings(settings, storage);
  }
  shared = { flags, save };
  notify();
}

export function useCampaignFlags(): CampaignFlags {
  const settings = useCampaignSettings();
  const flags = useSyncExternalStore(subscribe, sharedFlags, sharedFlags);
  return useMemo(() => flags ?? settingsFlags(settings), [flags, settings]);
}

/** Choose a campaign's flag, or follow the shown country with null. Returns a refusal
 * now; with shared settings the save completes later and `report` hears its outcome. */
export function saveCampaignFlag(campaignKey: string, tag: string | null, report?: (error: string | null) => void, storage?: Storage): string | null {
  if (shared) {
    const flags = { ...shared.flags };
    if (tag === null) delete flags[campaignKey]; else flags[campaignKey] = tag;
    shared = { ...shared, flags };
    notify();
    void shared.save(campaignKey, tag).then(() => report?.(null), () => report?.('The campaign flag could not be saved; it applies until the app closes.'));
    return null;
  }
  return saveCampaignSettings(setCampaignFlag(currentCampaignSettings(storage), campaignKey, tag), storage);
}
