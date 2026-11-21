import { useMemo, useSyncExternalStore } from 'react';
import { currentCampaignSettings, saveCampaignSettings, setCampaignFlag, useCampaignSettings } from './campaignSettings.ts';
import type { CampaignSettings } from './campaignSettings.ts';

export type CampaignFlags = Record<string, string>;
type Save = (campaignKey: string, tag: string | null) => Promise<void>;
type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

let shared: { flags: CampaignFlags; save: Save } | null = null;
let migrating: Set<string> | null = null;
let saving: Promise<unknown> = Promise.resolve();
const listeners = new Set<() => void>();
const notify = () => { for (const listener of listeners) listener(); };
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
const sharedFlags = () => shared?.flags ?? null;

export function settingsFlags(settings: CampaignSettings): CampaignFlags {
  return Object.fromEntries(Object.entries(settings.campaigns).flatMap(([key, entry]) => entry.flag ? [[key, entry.flag]] : []));
}

function inOrder(save: Save): Save {
  return (campaignKey, tag) => {
    const saved = saving.then(() => save(campaignKey, tag));
    saving = saved.catch(() => undefined);
    return saved;
  };
}

export async function shareCampaignFlags(read: CampaignFlags | Promise<CampaignFlags>, save: Save, storage?: Storage): Promise<void> {
  const early = new Set<string>();
  migrating = early;
  if (shared) { shared = null; notify(); }
  let stored: CampaignFlags;
  try {
    stored = await read;
  } catch (error) {
    if (migrating === early) migrating = null;
    throw error;
  }
  const local = settingsFlags(currentCampaignSettings(storage));
  const pending: [string, string | null][] = [
    ...Object.entries(local).filter(([key]) => !Object.hasOwn(stored, key) || early.has(key)),
    ...[...early].filter(key => !Object.hasOwn(local, key) && Object.hasOwn(stored, key)).map((key): [string, null] => [key, null]),
  ];
  const ordered = inOrder(save), chosen = new Set<string>(), failed: string[] = [];
  const flags = { ...stored };
  for (const [key, tag] of pending) if (tag === null) delete flags[key]; else flags[key] = tag;
  shared = { flags, save: ordered };
  migrating = chosen;
  notify();
  try {
    for (const [key, tag] of pending) {
      if (chosen.has(key)) continue;
      try { await ordered(key, tag); } catch { failed.push(key); }
    }
  } finally {
    if (migrating === chosen) migrating = null;
  }
  if (Object.keys(local).length) {
    let settings = currentCampaignSettings(storage);
    for (const key of Object.keys(local)) if (!failed.includes(key)) settings = setCampaignFlag(settings, key, null);
    saveCampaignSettings(settings, storage);
  }
  if (failed.length) throw new Error(`${failed.length} campaign flags could not be saved to the shared settings.`);
}

export function useCampaignFlags(): CampaignFlags {
  const settings = useCampaignSettings();
  const flags = useSyncExternalStore(subscribe, sharedFlags, sharedFlags);
  return useMemo(() => flags ?? settingsFlags(settings), [flags, settings]);
}

export function saveCampaignFlag(campaignKey: string, tag: string | null, report?: (error: string | null) => void, storage?: Storage): string | null {
  migrating?.add(campaignKey);
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
