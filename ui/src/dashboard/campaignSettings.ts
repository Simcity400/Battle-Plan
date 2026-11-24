import { useSyncExternalStore } from 'react';
import { normalizeColor } from '../shell/appearance/color.ts';
import { opaqueTag } from './validation.ts';

export interface FactionStyle { colour?: string; name?: string }
export interface FactionColourSet { id: string; name: string; entries: { faction: string; colour: string; name?: string }[] }
export interface CampaignSettings {
  version: 1;
  campaigns: Record<string, { factions: Record<string, FactionStyle>; flag?: string; counterCountries?: Record<string, string[]> }>;
  shared: Record<string, { colour: string }>;
  sets: FactionColourSet[];
}

export type ColourScope = 'all' | 'campaign';

export function factionIdentity(recordedName: string | null, founder: string | null): string | null {
  const key = recordedName ? JSON.stringify(['name', recordedName]) : founder ? JSON.stringify(['leader', founder]) : null;
  return key && key.length <= MAX_IDENTITY ? key : null;
}

const KEY = 'battleplan.campaign-settings.v1';
export const MAX_CAMPAIGN_SETTINGS_STORAGE = 2_000_000;
const MAX_TEXT = 160;
const MAX_IDENTITY = 400;
const MAX_SCOPE = 400;
const EMPTY: CampaignSettings = { version: 1, campaigns: {}, shared: {}, sets: [] };
type Storage = Pick<globalThis.Storage, 'getItem' | 'setItem'>;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value != null && !Array.isArray(value);
const text = (value: unknown): value is string => typeof value === 'string' && value.trim().length > 0 && value.length <= MAX_TEXT;
const safeKey = (key: string, pattern: RegExp) => pattern.test(key) && !['__proto__', 'constructor', 'prototype'].includes(key);
const colour = (value: unknown): string | undefined => {
  const hex = typeof value === 'string' ? normalizeColor(value) : null;
  return hex && /^#[0-9a-f]{6}$/.test(hex) ? hex : undefined;
};

const tagList = (value: unknown): string[] | null => Array.isArray(value) ? [...new Set(value.filter(opaqueTag))] : null;

function counterLists(value: unknown): Record<string, string[]> | null {
  if (!object(value)) return null;
  const lists: Record<string, string[]> = {};
  for (const [scope, entry] of Object.entries(value)) {
    const tags = scope.length <= MAX_SCOPE && safeKey(scope, /^./) ? tagList(entry) : null;
    if (tags) lists[scope] = tags;
  }
  return Object.keys(lists).length ? lists : null;
}

function style(value: unknown): FactionStyle | null {
  if (!object(value)) return null;
  const result: FactionStyle = {};
  if (value.colour !== undefined) { const hex = colour(value.colour); if (!hex) return null; result.colour = hex; }
  if (value.name !== undefined) { if (!text(value.name)) return null; result.name = value.name.trim(); }
  return result;
}

export function parseCampaignSettings(input: unknown): CampaignSettings {
  if (!object(input) || input.version !== 1) return structuredClone(EMPTY);
  const campaigns: CampaignSettings['campaigns'] = {};
  for (const [key, value] of Object.entries(object(input.campaigns) ? input.campaigns : {})) {
    if (!safeKey(key, /^[0-9a-f]{1,128}$/) || !object(value) || !object(value.factions)) continue;
    const factions: Record<string, FactionStyle> = {};
    for (const [id, entry] of Object.entries(value.factions)) {
      const parsed = safeKey(id, /^\d{1,20}$/) ? style(entry) : null;
      if (parsed && (parsed.colour || parsed.name)) factions[id] = parsed;
    }
    const counters = counterLists(value.counterCountries);
    campaigns[key] = { factions, ...(opaqueTag(value.flag) ? { flag: value.flag } : {}), ...(counters ? { counterCountries: counters } : {}) };
  }
  const shared: CampaignSettings['shared'] = {};
  for (const [key, value] of Object.entries(object(input.shared) ? input.shared : {})) {
    const hex = object(value) ? colour(value.colour) : undefined;
    if (hex && key.length <= MAX_IDENTITY && !['__proto__', 'constructor', 'prototype'].includes(key)) shared[key] = { colour: hex };
  }
  const sets: FactionColourSet[] = [];
  for (const value of Array.isArray(input.sets) ? input.sets : []) {
    if (!object(value) || !safeKey(String(value.id), /^[a-zA-Z0-9_-]{1,64}$/) || !text(value.name) || !Array.isArray(value.entries)) continue;
    const entries = value.entries.flatMap(entry => {
      if (!object(entry) || !text(entry.faction)) return [];
      const hex = colour(entry.colour);
      if (!hex) return [];
      return [{ faction: entry.faction.trim(), colour: hex, ...(text(entry.name) ? { name: entry.name.trim() } : {}) }];
    });
    if (!sets.some(set => set.id === value.id)) sets.push({ id: String(value.id), name: value.name.trim(), entries });
  }
  return { version: 1, campaigns, shared, sets };
}

export function setFactionColour(settings: CampaignSettings, scope: ColourScope, campaign: string, faction: string,
  identity: string | null, hex: string | null): CampaignSettings {
  if (scope === 'campaign' || identity == null) return setFactionStyle(settings, campaign, faction, { colour: hex ?? undefined });
  const value = hex == null ? null : colour(hex);
  if (hex != null && !value) return settings;
  const shared = { ...settings.shared };
  if (value) shared[identity] = { colour: value };
  else delete shared[identity];
  const next = { ...settings, shared };
  return settings.campaigns[campaign]?.factions[faction]?.colour ? setFactionStyle(next, campaign, faction, { colour: undefined }) : next;
}

export function chosenColour(settings: CampaignSettings, campaign: string, faction: string, identity: string | null): string | undefined {
  return settings.campaigns[campaign]?.factions[faction]?.colour ?? (identity != null ? settings.shared[identity]?.colour : undefined);
}

export function setFactionStyle(settings: CampaignSettings, campaign: string, faction: string, patch: FactionStyle | null): CampaignSettings {
  const factions = { ...settings.campaigns[campaign]?.factions };
  const next: FactionStyle = patch === null ? {} : { ...factions[faction], ...patch };
  if (next.colour === undefined) delete next.colour;
  if (next.colour !== undefined) {
    const hex = colour(next.colour);
    if (!hex) return settings;
    next.colour = hex;
  }
  if (next.name !== undefined) { if (text(next.name)) next.name = next.name.trim(); else delete next.name; }
  if (next.colour || next.name) factions[faction] = next; else delete factions[faction];
  return { ...settings, campaigns: { ...settings.campaigns, [campaign]: { ...settings.campaigns[campaign], factions } } };
}

export function setCampaignFlag(settings: CampaignSettings, campaign: string, tag: string | null): CampaignSettings {
  if (!safeKey(campaign, /^[0-9a-f]{1,128}$/) || tag !== null && !opaqueTag(tag)) return settings;
  const entry = { ...(settings.campaigns[campaign] ?? { factions: {} }) };
  if (tag === null) delete entry.flag; else entry.flag = tag;
  return { ...settings, campaigns: { ...settings.campaigns, [campaign]: entry } };
}

export function counterCountries(settings: CampaignSettings, campaign: string, scope: string): string[] | undefined {
  return Object.hasOwn(settings.campaigns, campaign) ? settings.campaigns[campaign].counterCountries?.[scope] : undefined;
}

export function setCounterCountries(settings: CampaignSettings, campaign: string, scope: string, tags: readonly string[]): CampaignSettings {
  if (!safeKey(campaign, /^[0-9a-f]{1,128}$/) || scope.length > MAX_SCOPE || !safeKey(scope, /^./)) return settings;
  const entry = settings.campaigns[campaign] ?? { factions: {} };
  return { ...settings, campaigns: { ...settings.campaigns, [campaign]: { ...entry, counterCountries: { ...entry.counterCountries, [scope]: tagList(tags) ?? [] } } } };
}

let current: CampaignSettings | null = null;
const listeners = new Set<() => void>();

export function loadCampaignSettings(storage?: Storage): CampaignSettings {
  try {
    const raw = (storage ?? localStorage).getItem(KEY);
    return raw && raw.length <= MAX_CAMPAIGN_SETTINGS_STORAGE ? parseCampaignSettings(JSON.parse(raw)) : structuredClone(EMPTY);
  } catch {
    return structuredClone(EMPTY);
  }
}

export function saveCampaignSettings(settings: CampaignSettings, storage?: Storage): string | null {
  const safe = parseCampaignSettings(settings);
  current = safe;
  for (const listener of listeners) listener();
  try {
    const raw = JSON.stringify(safe);
    if (raw.length > MAX_CAMPAIGN_SETTINGS_STORAGE) throw new Error('Campaign settings exceed the settings storage budget.');
    (storage ?? localStorage).setItem(KEY, raw);
    return null;
  } catch {
    return 'Couldn’t save campaign settings on this device. They apply until the app closes.';
  }
}

const snapshot = () => (current ??= loadCampaignSettings());
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

export function currentCampaignSettings(storage?: Storage): CampaignSettings {
  return current ??= loadCampaignSettings(storage);
}

export function useCampaignSettings(): CampaignSettings {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function factionStylesKey(settings: CampaignSettings, campaign: string | null): string {
  if (campaign === null) return '';
  const own = settings.campaigns[campaign]?.factions;
  const shared = Object.keys(settings.shared).length ? settings.shared : undefined;
  return own || shared ? JSON.stringify([own ?? null, shared ?? null]) : '';
}

export function useFactionStylesKey(campaign: string | null): string {
  const read = () => factionStylesKey(snapshot(), campaign);
  return useSyncExternalStore(subscribe, read, read);
}
