import { useSyncExternalStore } from 'react';
import { normalizeColor } from '../shell/appearance/color.ts';

/** Per-campaign app settings, kept with the app (never in the recording): each
 * recorded faction's colour and display name, and saved colour sets that can be
 * applied to other campaigns, matched by faction name. Campaigns are keyed by the
 * recorded campaign identity, so every recording of a campaign shares them. */

export interface FactionStyle { colour?: string; name?: string }
export interface FactionColourSet { id: string; name: string; entries: { faction: string; colour: string; name?: string }[] }
export interface CampaignSettings {
  version: 1;
  campaigns: Record<string, { factions: Record<string, FactionStyle> }>;
  sets: FactionColourSet[];
}

const KEY = 'hoi4-replay.campaign-settings.v1';
export const MAX_CAMPAIGNS = 256;
export const MAX_FACTIONS = 256;
export const MAX_SETS = 64;
const MAX_TEXT = 160;
const EMPTY: CampaignSettings = { version: 1, campaigns: {}, sets: [] };
type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0 && value.length <= MAX_TEXT;
const safeKey = (key: string, pattern: RegExp) => pattern.test(key) && !['__proto__', 'constructor', 'prototype'].includes(key);
const colour = (value: unknown): string | undefined => {
  const hex = typeof value === 'string' ? normalizeColor(value) : null;
  return hex && /^#[0-9a-f]{6}$/.test(hex) ? hex : undefined;
};

function style(value: unknown): FactionStyle | null {
  if (!object(value)) return null;
  const result: FactionStyle = {};
  if (value.colour !== undefined) { const hex = colour(value.colour); if (!hex) return null; result.colour = hex; }
  if (value.name !== undefined) { if (!text(value.name)) return null; result.name = value.name.trim(); }
  return result;
}

/** Allowlist parse; anything malformed is dropped rather than trusted. */
export function parseCampaignSettings(input: unknown): CampaignSettings {
  if (!object(input) || input.version !== 1) return structuredClone(EMPTY);
  const campaigns: CampaignSettings['campaigns'] = {};
  for (const [key, value] of Object.entries(object(input.campaigns) ? input.campaigns : {}).slice(0, MAX_CAMPAIGNS)) {
    if (!safeKey(key, /^[0-9a-f]{1,128}$/) || !object(value) || !object(value.factions)) continue;
    const factions: Record<string, FactionStyle> = {};
    for (const [id, entry] of Object.entries(value.factions).slice(0, MAX_FACTIONS)) {
      const parsed = safeKey(id, /^\d{1,20}$/) ? style(entry) : null;
      if (parsed && (parsed.colour || parsed.name)) factions[id] = parsed;
    }
    campaigns[key] = { factions };
  }
  const sets: FactionColourSet[] = [];
  for (const value of Array.isArray(input.sets) ? input.sets.slice(0, MAX_SETS) : []) {
    if (!object(value) || !safeKey(String(value.id), /^[a-zA-Z0-9_-]{1,64}$/) || !text(value.name) || !Array.isArray(value.entries)) continue;
    const entries = value.entries.slice(0, MAX_FACTIONS).flatMap(entry => {
      if (!object(entry) || !text(entry.faction)) return [];
      const hex = colour(entry.colour);
      if (!hex) return [];
      return [{ faction: entry.faction.trim(), colour: hex, ...(text(entry.name) ? { name: entry.name.trim() } : {}) }];
    });
    if (!sets.some(set => set.id === value.id)) sets.push({ id: String(value.id), name: value.name.trim(), entries });
  }
  return { version: 1, campaigns, sets };
}

/** Sets or clears one faction's colour or display name. */
export function setFactionStyle(settings: CampaignSettings, campaign: string, faction: string, patch: FactionStyle | null): CampaignSettings {
  const factions = { ...settings.campaigns[campaign]?.factions };
  // A new faction past the limit is refused here, rather than dropped when saved.
  if (patch !== null && !(faction in factions) && Object.keys(factions).length >= MAX_FACTIONS) return settings;
  const next: FactionStyle = patch === null ? {} : { ...factions[faction], ...patch };
  if (next.colour !== undefined) {
    const hex = colour(next.colour);
    if (!hex) return settings;
    next.colour = hex;
  }
  if (next.name !== undefined) { if (text(next.name)) next.name = next.name.trim(); else delete next.name; }
  if (next.colour || next.name) factions[faction] = next; else delete factions[faction];
  return { ...settings, campaigns: { ...settings.campaigns, [campaign]: { factions } } };
}

/** Saves a campaign's chosen colours as a named set, keyed by faction name. */
export function saveColourSet(settings: CampaignSettings, id: string, name: string, campaign: string,
  factions: readonly { id: string; name: string | null }[], colours: ReadonlyMap<string, string>): CampaignSettings {
  const chosen = settings.campaigns[campaign]?.factions ?? {};
  const entries = factions.flatMap(faction => {
    const shown = chosen[faction.id]?.name ?? faction.name;
    const hex = chosen[faction.id]?.colour ?? colours.get(faction.id);
    return shown && hex ? [{ faction: faction.name ?? shown, colour: hex, ...(chosen[faction.id]?.name ? { name: chosen[faction.id].name } : {}) }] : [];
  });
  const set = { id, name: name.trim().slice(0, MAX_TEXT), entries };
  return { ...settings, sets: [...settings.sets.filter(item => item.id !== id), set].slice(-MAX_SETS) };
}

/** Applies a saved set to a campaign: each recorded faction whose name matches an
 * entry (ignoring case) takes its colour and display name. Returns how many matched. */
export function applyColourSet(settings: CampaignSettings, set: FactionColourSet, campaign: string,
  factions: readonly { id: string; name: string | null }[]): { settings: CampaignSettings; matched: number } {
  let next = settings;
  let matched = 0;
  for (const faction of factions) {
    const entry = faction.name && set.entries.find(item => item.faction.toLowerCase() === faction.name!.toLowerCase());
    if (!entry) continue;
    next = setFactionStyle(next, campaign, faction.id, { colour: entry.colour, ...(entry.name ? { name: entry.name } : {}) });
    matched++;
  }
  return { settings: next, matched };
}

let current: CampaignSettings | null = null;
const listeners = new Set<() => void>();

export function loadCampaignSettings(storage?: Storage): CampaignSettings {
  try {
    const raw = (storage ?? localStorage).getItem(KEY);
    return raw && raw.length <= 2_000_000 ? parseCampaignSettings(JSON.parse(raw)) : structuredClone(EMPTY);
  } catch {
    return structuredClone(EMPTY);
  }
}

/** Saves and tells every open view. Returns an error message if the device refused. */
export function saveCampaignSettings(settings: CampaignSettings, storage?: Storage): string | null {
  const safe = parseCampaignSettings(settings);
  current = safe;
  for (const listener of listeners) listener();
  try {
    (storage ?? localStorage).setItem(KEY, JSON.stringify(safe));
    return null;
  } catch {
    return 'Faction colours could not be saved on this device; they apply until the app closes.';
  }
}

const snapshot = () => (current ??= loadCampaignSettings());
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

export function useCampaignSettings(): CampaignSettings {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
