import { useSyncExternalStore } from 'react';
import { normalizeColor } from '../shell/appearance/color.ts';
import { opaqueTag } from './validation.ts';

/** Faction settings, kept with the app (never in the recording). A faction colour the
 * user sets applies in every campaign (`shared`, keyed by the faction's identity,
 * `factionIdentity`) until changed again; a campaign can override it for itself
 * (`campaigns`, keyed by the recorded campaign identity and faction id, so every
 * recording of a campaign shares them), with its own display names. Saved colour sets
 * can still be applied to other campaigns, matched by faction name. */

export interface FactionStyle { colour?: string; name?: string }
export interface FactionColourSet { id: string; name: string; entries: { faction: string; colour: string; name?: string }[] }
export interface CampaignSettings {
  version: 1;
  campaigns: Record<string, { factions: Record<string, FactionStyle>; flag?: string }>;
  /** Colours chosen for every campaign, by faction identity (`factionIdentity`). */
  shared: Record<string, { colour: string }>;
  sets: FactionColourSet[];
}

/** Where a colour the user picks applies: every campaign (the default), or only this one. */
export type ColourScope = 'all' | 'campaign';

/** A faction's identity across campaigns, for colours set for every campaign: its
 * literal name exactly as recorded (unchanged by translation, so it holds
 * across languages and content). An invalid UTF-8 literal is refused and has an
 * empty recorded name, so its identity also falls back to the tag of the leader
 * it had when first recorded. Null when neither is known: such a faction's colour can
 * only be set for its campaign.
 *
 * This colour key has limits: two factions recorded
 * under one literal name share a colour; and a faction formed from a template has no recorded
 * name, so it is known by its first leader's tag, which is not stable across
 * campaigns (the same template led first by another country is not matched, and
 * another faction first led by that tag is). */
export function factionIdentity(recordedName: string | null, founder: string | null): string | null {
  const key = recordedName ? JSON.stringify(['name', recordedName]) : founder ? JSON.stringify(['leader', founder]) : null;
  return key && key.length <= MAX_IDENTITY ? key : null;
}

const KEY = 'hoi4-replay.campaign-settings.v1';
export const MAX_CAMPAIGNS = 256;
export const MAX_FACTIONS = 256;
export const MAX_SETS = 64;
const MAX_TEXT = 160;
const MAX_IDENTITY = 400;
const EMPTY: CampaignSettings = { version: 1, campaigns: {}, shared: {}, sets: [] };
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
    campaigns[key] = { factions, ...(opaqueTag(value.flag) ? { flag: value.flag } : {}) };
  }
  const shared: CampaignSettings['shared'] = {};
  for (const [key, value] of Object.entries(object(input.shared) ? input.shared : {}).slice(0, MAX_FACTIONS * 4)) {
    const hex = object(value) ? colour(value.colour) : undefined;
    if (hex && key.length <= MAX_IDENTITY && !['__proto__', 'constructor', 'prototype'].includes(key)) shared[key] = { colour: hex };
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
  return { version: 1, campaigns, shared, sets };
}

/** Sets or clears a faction's colour. For every campaign (`all`): it is stored by the
 * faction's identity and this campaign's own colour for it is dropped, so the choice
 * shows here too; a faction with no identity falls back to this campaign alone. For
 * this campaign only: it overrides the colour for every campaign here. `null` clears
 * the colour at that scope. */
export function setFactionColour(settings: CampaignSettings, scope: ColourScope, campaign: string, faction: string,
  identity: string | null, hex: string | null): CampaignSettings {
  if (scope === 'campaign' || identity == null) return setFactionStyle(settings, campaign, faction, { colour: hex ?? undefined });
  const value = hex == null ? null : colour(hex);
  if (hex != null && !value) return settings;
  const shared = { ...settings.shared };
  if (value) {
    if (!(identity in shared) && Object.keys(shared).length >= MAX_FACTIONS * 4) return settings;
    shared[identity] = { colour: value };
  } else delete shared[identity];
  const next = { ...settings, shared };
  return settings.campaigns[campaign]?.factions[faction]?.colour ? setFactionStyle(next, campaign, faction, { colour: undefined }) : next;
}

/** A faction's chosen colour, if any: this campaign's own, else the one for every campaign. */
export function chosenColour(settings: CampaignSettings, campaign: string, faction: string, identity: string | null): string | undefined {
  return settings.campaigns[campaign]?.factions[faction]?.colour ?? (identity != null ? settings.shared[identity]?.colour : undefined);
}

/** Sets or clears one faction's colour or display name. */
export function setFactionStyle(settings: CampaignSettings, campaign: string, faction: string, patch: FactionStyle | null): CampaignSettings {
  const factions = { ...settings.campaigns[campaign]?.factions };
  // A new faction past the limit is refused here, rather than dropped when saved.
  if (patch !== null && !(faction in factions) && Object.keys(factions).length >= MAX_FACTIONS) return settings;
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

/** A recorded tag overrides the sidebar flag; null follows the shown country. */
export function setCampaignFlag(settings: CampaignSettings, campaign: string, tag: string | null): CampaignSettings {
  if (!safeKey(campaign, /^[0-9a-f]{1,128}$/) || tag !== null && !opaqueTag(tag)) return settings;
  if (!(campaign in settings.campaigns) && Object.keys(settings.campaigns).length >= MAX_CAMPAIGNS) return settings;
  const entry = { ...(settings.campaigns[campaign] ?? { factions: {} }) };
  if (tag === null) delete entry.flag; else entry.flag = tag;
  return { ...settings, campaigns: { ...settings.campaigns, [campaign]: entry } };
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
    return 'Campaign settings could not be saved on this device; they apply until the app closes.';
  }
}

const snapshot = () => (current ??= loadCampaignSettings());
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };

/** The settings in effect: the last saved, including a save this device refused to store. */
export function currentCampaignSettings(storage?: Storage): CampaignSettings {
  return current ??= loadCampaignSettings(storage);
}

export function useCampaignSettings(): CampaignSettings {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}
