import { runAt } from '../../charts/series.ts';
import type { Artwork, ResearchBreakdownReply, ResearchCoverage, ResearchEntry, ResearchReply, ResearchSlotNow, ResearchSlotState } from '../../generated/dashboard';
import type { ResearchSlotsReply } from '../../generated/dashboard';
import { artworkEntry, keepDrawableIcons } from '../../dashboard/artwork.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { researchGrants } from '../active-research/researchGrants.ts';
import type { ResearchData, ResearchHistory, ResearchSlotHistory } from './types';

export function researchAt(history: ResearchHistory, hour: number): ResearchSlotHistory[] {
  return history.slots.filter(s => s.availability[0]?.firstHour <= hour).map(slot => ({
    ...slot,
    availability: slot.availability.filter(c => c.firstHour <= hour).map(c => ({ ...c, lastHour: Math.min(c.lastHour, hour) })),
    states: slot.states.filter(s => s.firstHour <= hour).map(s => ({ ...s, lastHour: Math.min(s.lastHour, hour) })),
    progress: slot.progress.filter(p => p.firstHour <= hour).map(p => ({ ...p, lastHour: Math.min(p.lastHour, hour) })),
    entries: slot.entries.filter(e => e.firstObservedHour <= hour).map(e => ({ ...e,
      lastObservedHour: Math.min(e.lastObservedHour, hour),
      outcomeObservedHour: e.outcomeObservedHour != null && e.outcomeObservedHour <= hour ? e.outcomeObservedHour : null,
      endedByHour: e.endedByHour !== null && e.endedByHour <= hour ? e.endedByHour : null,
      outcome: e.endedByHour !== null && e.endedByHour <= hour
        ? e.outcomeObservedHour != null && e.outcomeObservedHour > hour ? 'unknown' : e.outcome : 'active',
    })),
  }));
}

export function withSlotReads(slots: readonly ResearchSlotHistory[], now: ResearchSlotsReply | undefined): ResearchSlotHistory[] {
  if (!now) return [...slots];
  const reads = new Map(now.slots.map(slot => [slot.index, slot]));
  return slots.map(slot => {
    const read = reads.get(slot.index);
    return read ? { ...slot, states: [read.state], progress: read.progress ? [read.progress] : [] } : { ...slot, states: [], progress: [] };
  });
}

export function withBreakdown(slots: readonly ResearchSlotHistory[], breakdown: ResearchBreakdownReply | undefined): ResearchSlotHistory[] {
  if (!breakdown) return [...slots];
  return slots.map(slot => {
    const parts = breakdown.parts.filter(part => part.slot === slot.index);
    if (!parts.length) return slot;
    const states = parts.flatMap(part => part.states).sort((a, b) => a.firstHour - b.firstHour);
    const progress = parts.flatMap(part => part.progress).sort((a, b) => a.firstHour - b.firstHour);
    return { ...slot, states, progress };
  });
}

export const completedBy = (entry: ResearchEntry) => entry.endedByHour !== null && entry.endedByHour > entry.lastObservedHour + 1;

export function researchEntryAt(slot: ResearchSlotHistory, state: ResearchSlotState, hour: number) {
  let low = 0, high = slot.entries.length;
  while (low < high) { const mid = (low + high) >>> 1; if (slot.entries[mid].firstObservedHour < (state.entryFirstObservedHour ?? -Infinity)) low = mid + 1; else high = mid; }
  for (; low < slot.entries.length && slot.entries[low].firstObservedHour === state.entryFirstObservedHour; low++) {
    const entry = slot.entries[low];
    if (entry.token === state.token && entry.databaseIndex === state.databaseIndex && entry.firstObservedHour <= hour && (entry.endedByHour === null || entry.endedByHour > hour))
      return { ...entry, lastObservedHour: Math.min(entry.lastObservedHour, hour), endedByHour: null, outcomeObservedHour: null, outcome: 'active' as const };
  }
  return null;
}

export function isUnjoinedResearch(state: ResearchSlotState | null): state is ResearchSlotState & { token: string } {
  return !!state?.token && state.entryFirstObservedHour === null;
}

export function researchStateAt(slot: ResearchSlotHistory, hour: number): ResearchSlotState | null {
  if (Number.isNaN(hour)) return null;
  if (!runAt(slot.availability, hour)) return null;
  const state = runAt(slot.states, hour);
  if (!state) return null;
  return state.details?.remainingDays == null ? state : { ...state, details: { ...state.details,
    remainingDays: Math.max(0, state.details.remainingDays - Math.floor((hour - state.firstHour) / 24)),
  } };
}

export function researchProgressAt(slot: ResearchSlotHistory, entryFirstObservedHour: number, hour: number): number | null {
  if (Number.isNaN(hour)) return null;
  const sample = runAt(slot.progress, hour);
  if (!sample?.baseDaysRaw || sample.entryFirstObservedHour !== entryFirstObservedHour) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  if (base <= 0n || done < 0n) return null;
  return done >= base ? 1 : Number(done * 10_000n / base) / 10_000;
}

export function researchRateAt(state: ResearchSlotState, hour: number): { rateRaw: string; penaltyRaw: string; floored: boolean } | null {
  const modifiers = state.details?.modifiers;
  if (!modifiers || state.aheadReductionYearsRaw === null) return null;
  const reduction = BigInt(state.aheadReductionYearsRaw) * 365n;
  const rounded = ((reduction < 0n ? -reduction : reduction) + 50000n) / 100000n * (reduction < 0n ? -1n : 1n);
  const ahead = BigInt(modifiers.year) * 365n - (BigInt(hour) - 43800000n) / 24n - rounded;
  const penalty = (ahead > 0n ? ahead : 0n) * BigInt(modifiers.aheadFactorRaw) / 365n;
  if (100000n + penalty <= 0n) return null;
  const rate = (100000n + BigInt(modifiers.totalBonusRaw)) * 100000n / (100000n + penalty);
  const minimum = BigInt(modifiers.minimumSpeedRaw);
  return { rateRaw: (rate > minimum ? rate : minimum).toString(), penaltyRaw: penalty.toString(), floored: rate < minimum };
}

export function researchLatestProgress(slot: ResearchSlotHistory, state: ResearchSlotState, hour: number): { fraction: number; hour: number } | null {
  if (state.entryFirstObservedHour === null) return null;
  const samples = slot.progress.filter(sample => sample.entryFirstObservedHour === state.entryFirstObservedHour);
  let low = 0, high = samples.length;
  while (low < high) { const mid = (low + high) >>> 1; if (samples[mid].firstHour <= hour) low = mid + 1; else high = mid; }
  const sample = samples[low - 1];
  if (!sample?.baseDaysRaw) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  if (base <= 0n || done < 0n) return null;
  return { fraction: done >= base ? 1 : Number(done * 10_000n / base) / 10_000, hour: Math.min(sample.lastHour, hour) };
}

export interface ResearchFolderLook { key: string; name: string; colour: string; order: number }

export type ResearchUse = string | readonly [token: string, databaseIndex: number];

export type ResearchFolderLooks = ReadonlyMap<string, ResearchFolderLook>
  & { readonly definitions?: ReadonlyMap<string, ReadonlyMap<number, ResearchFolderLook>> };

export function researchFolderLooks(reply: Pick<ResearchReply, 'folders' | 'technologyFolders'>, tokens: Iterable<ResearchUse>,
  palette: readonly string[]): Map<string, ResearchFolderLook> & { definitions: Map<string, Map<number, ResearchFolderLook>> } {
  const byToken = new Map<string, string | null>(), byDefinition = new Map<string, Map<number, string | null>>();
  for (const item of reply.technologyFolders) {
    if (item.databaseIndex == null) byToken.set(item.token, item.folder);
    else byDefinition.set(item.token, (byDefinition.get(item.token) ?? new Map<number, string | null>()).set(item.databaseIndex, item.folder));
  }
  const folderOf = (use: ResearchUse) => {
    if (typeof use === 'string') return byToken.get(use);
    const [token, index] = use, own = byDefinition.get(token);
    return own?.has(index) ? own.get(index) : byToken.get(token);
  };
  const order = new Map(reply.folders.map((folder, index) => [folder.key, index]));
  const names = new Map(reply.folders.map(folder => [folder.key, folder.name]));
  const wanted = [...tokens];
  const shown = [...new Set(wanted.flatMap(use => folderOf(use) ?? []))]
    .sort((a, b) => (order.get(a) ?? Infinity) - (order.get(b) ?? Infinity) || (a < b ? -1 : a > b ? 1 : 0));
  const looks = new Map(shown.map((key, index): [string, ResearchFolderLook] => [key, { key, order: index,
    name: contentTitle(names.get(key), key, 'Folder name unavailable'), colour: palette.length ? palette[index % palette.length] : 'currentColor' }]));
  const byUse = new Map<string, ResearchFolderLook>(), definitions = new Map<string, Map<number, ResearchFolderLook>>();
  for (const use of wanted) {
    const key = folderOf(use), look = key ? looks.get(key) : undefined;
    if (!look) continue;
    if (typeof use === 'string') byUse.set(use, look);
    else definitions.set(use[0], (definitions.get(use[0]) ?? new Map<number, ResearchFolderLook>()).set(use[1], look));
  }
  return Object.assign(byUse, { definitions });
}

export function researchFolderAt(looks: ResearchFolderLooks, token: string,
  databaseIndex?: number | null): ResearchFolderLook | undefined {
  return databaseIndex == null ? looks.get(token) : looks.definitions?.get(token)?.get(databaseIndex);
}

export type ResearchArtwork = Map<string, Artwork> & { readonly definitions: ReadonlyMap<string, ReadonlyMap<number, Artwork>> };

export function researchArtwork(reply: Pick<ResearchReply, 'artwork' | 'definitions' | 'history'> | null | undefined,
  images: ReadonlyMap<string, Artwork>): ResearchArtwork {
  const artwork = new Map<string, Artwork>(), definitions = new Map<string, Map<number, Artwork>>();
  for (const art of reply?.artwork ?? []) artwork.set(art.token, { ...art, icon: images.get(art.token)?.icon ?? art.icon });
  for (const [token, [only, ...more]] of researchRecordedTemplates(reply?.history)) {
    const art = artwork.get(token);
    if (art && only != null && !more.length) definitions.set(token, new Map([[only, art]]));
  }
  for (const definition of reply?.definitions ?? []) {
    const art = artwork.get(definition.token);
    definitions.set(definition.token, (definitions.get(definition.token) ?? new Map<number, Artwork>()).set(definition.databaseIndex,
      { token: definition.token, name: definition.name, description: definition.description, baseDurationDays: art?.baseDurationDays ?? null,
        icon: art?.icon ?? images.get(definition.token)?.icon ?? null }));
  }
  return Object.assign(artwork, { definitions });
}

export function researchRecordedTemplates(history: { slots: readonly { entries: readonly { token: string; databaseIndex: number }[] }[] } | null | undefined): Map<string, (number | null)[]> {
  const recorded = new Map<string, Set<number>>();
  for (const slot of history?.slots ?? []) for (const entry of slot.entries)
    recorded.set(entry.token, (recorded.get(entry.token) ?? new Set<number>()).add(entry.databaseIndex));
  return new Map([...recorded].map(([token, uses]) => [token, [...uses].sort((a, b) => a - b)]));
}

export function researchArt(artwork: ResearchArtwork, token: string, databaseIndex?: number | null): Artwork | undefined {
  return databaseIndex == null ? artwork.get(token) : artwork.definitions.get(token)?.get(databaseIndex);
}

export function researchReadName(artwork: ResearchArtwork, read: ResearchSlotNow): string | null | undefined {
  const { token, databaseIndex } = read.state;
  if (token === null) return undefined;
  return isUnjoinedResearch(read.state) ? read.definition?.name : researchArt(artwork, token, databaseIndex)?.name;
}

export function historyTokens(history: ResearchHistory): ResearchUse[] {
  const tokens = new Set<string>(), definitions = new Map<string, Set<number>>();
  for (const slot of history.slots) {
    for (const entry of slot.entries) definitions.set(entry.token, (definitions.get(entry.token) ?? new Set<number>()).add(entry.databaseIndex));
    for (const state of slot.states) if (state.token && !definitions.has(state.token)) tokens.add(state.token);
  }
  return [...tokens, ...[...definitions].flatMap(([token, indices]) => [...indices].map(index => [token, index] as const))];
}

export function researchFolderList(looks: ResearchFolderLooks): ResearchFolderLook[] {
  const seen = new Map<string, ResearchFolderLook>();
  const all = [...looks.values(), ...[...looks.definitions?.values() ?? []].flatMap(byIndex => [...byIndex.values()])];
  for (const look of all) if (!seen.has(look.key)) seen.set(look.key, look);
  return [...seen.values()].sort((a, b) => a.order - b.order);
}

export function researchArtworkTokens(slots: readonly ResearchSlotHistory[] | null, hour: number | undefined, moment: boolean): string[] {
  if (!slots || hour === undefined) return [];
  const ordered = [...slots].sort((a, b) => a.index - b.index), tokens: string[] = [];
  if (moment) for (const slot of ordered) {
    const current = researchStateAt(slot, hour);
    const entry = current && slot.entries.find(entry => entry.outcome === 'active'
      && entry.firstObservedHour === current.entryFirstObservedHour && entry.token === current.token && entry.databaseIndex === current.databaseIndex);
    if (entry) tokens.push(entry.token);
    else if (isUnjoinedResearch(current)) tokens.push(current.token);
  }
  for (const slot of ordered) for (const entry of slot.entries) if (entry.outcome === 'completed') tokens.push(entry.token);
  return [...new Set(tokens)];
}

const hour = (n: unknown): n is number => Number.isInteger(n) && (n as number) >= -2147483648 && (n as number) <= 2147483647;
const fixed = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,19}$/.test(value)
  && BigInt(value) >= -9223372036854775808n && BigInt(value) <= 9223372036854775807n;
const templateIndex = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= 1 && (value as number) <= 4294967295;
const text = (value: unknown, limit = 4096) => value === null || (typeof value === 'string' && value.length <= limit);
const intervals = (spans: ResearchCoverage[]) => {
  let last = -Infinity;
  return Array.isArray(spans) && spans.every(s => {
    const valid = hour(s.firstHour) && hour(s.lastHour) && s.firstHour > last && s.lastHour >= s.firstHour;
    last = s.lastHour;
    return valid;
  });
};

function validState(state: ResearchSlotState): boolean {
  const details = state.details;
  return hour(state.firstHour) && hour(state.lastHour) && state.lastHour >= state.firstHour
    && (state.token === null || (typeof state.token === 'string' && state.token.length > 0 && state.token.length <= 512))
    && (state.databaseIndex === null ? state.token === null : state.token !== null && templateIndex(state.databaseIndex))
    && (state.entryFirstObservedHour === null || hour(state.entryFirstObservedHour))
    && fixed(state.speedMultiplierRaw) && fixed(state.bankedDaysRaw) && fixed(state.appliedDaysRaw)
    && (state.aheadReductionYearsRaw === null || fixed(state.aheadReductionYearsRaw))
    && (details === null || (
      (details.remainingDays === null ? details.estimateRaw === null : Number.isInteger(details.remainingDays) && details.remainingDays >= 0 && details.remainingDays <= 4294967295
        && fixed(details.estimateRaw) && BigInt(details.estimateRaw) <= BigInt(details.remainingDays) * 100_000n
        && BigInt(details.estimateRaw) > BigInt(details.remainingDays) * 100_000n - 100_000n)
      && (details.modifiers === null || (hour(details.modifiers.year)
        && fixed(details.modifiers.totalBonusRaw) && fixed(details.modifiers.aheadFactorRaw) && fixed(details.modifiers.minimumSpeedRaw)
        && Array.isArray(details.modifiers.components) && details.modifiers.components.length === 8
        && new Set(details.modifiers.components.map(c => c.kind)).size === 8
        && details.modifiers.components.every(c => ['country', 'category', 'designTeam', 'experience', 'license', 'scientist', 'applied', 'sharing'].includes(c.kind)
          && fixed(c.raw) && Array.isArray(c.sources) && c.sources.every(source => fixed(source.raw) && text(source.token, 512) && text(source.name)))))
      && (details.bonuses === null || (Array.isArray(details.bonuses)
        && details.bonuses.every(b => Number.isInteger(b.id) && b.id >= -2147483648 && b.id <= 2147483647 && text(b.token, 512) && text(b.name)
          && (b.speedBonusRaw === null || fixed(b.speedBonusRaw)) && (b.costReductionRaw === null || fixed(b.costReductionRaw))
          && (b.aheadReductionYearsRaw === null || fixed(b.aheadReductionYearsRaw)))))));
}

const validProgress = (sample: { firstHour: number; lastHour: number; entryFirstObservedHour: number | null; baseDaysRaw: string | null; progressRaw: string }) =>
  hour(sample.firstHour) && hour(sample.lastHour) && sample.lastHour >= sample.firstHour
  && (sample.entryFirstObservedHour === null || hour(sample.entryFirstObservedHour))
  && fixed(sample.progressRaw) && (sample.baseDaysRaw === null || (fixed(sample.baseDaysRaw) && BigInt(sample.baseDaysRaw) > 0n));

export function validateResearchReply(input: unknown): ResearchData {
  const reply = input as ResearchReply;
  let index = -1;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.artwork) && Array.isArray(reply.history?.slots)
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && intervals(reply.history.coverage) && reply.history.slots.every(s => {
      let last = -Infinity;
      const valid = Number.isInteger(s.index) && s.index > index && s.availability.length > 0 && intervals(s.availability)
        && Array.isArray(s.entries) && s.entries.every(e => {
          const ok = typeof e.token === 'string' && e.token.length > 0 && e.token.length <= 512 && Number.isInteger(e.level)
            && Number.isInteger(e.technologyHandle) && e.technologyHandle >= 0 && e.technologyHandle <= 4294967295
            && templateIndex(e.databaseIndex)
            && hour(e.continuityStartHour) && e.continuityStartHour <= e.firstObservedHour
            && hour(e.firstObservedHour) && e.firstObservedHour >= last && hour(e.lastObservedHour) && e.lastObservedHour >= e.firstObservedHour
            && (e.startedAfterHour === null || (hour(e.startedAfterHour) && e.startedAfterHour < e.firstObservedHour))
            && (e.outcomeObservedHour === null || (hour(e.outcomeObservedHour) && e.endedByHour !== null && e.outcomeObservedHour >= e.endedByHour))
            && ['active', 'completed', 'switched', 'unknown'].includes(e.outcome)
            && (e.endedByHour === null ? e.outcome === 'active' : hour(e.endedByHour) && e.endedByHour >= e.lastObservedHour && e.outcome !== 'active');
          last = e.endedByHour ?? Infinity;
          return ok;
        });
      index = s.index;
      return valid;
    }) && reply.artwork.every(artworkEntry)
    && Array.isArray(reply.folders) && reply.folders.every(folder => typeof folder?.key === 'string' && folder.key.length > 0 && text(folder.name))
    && Array.isArray(reply.technologyFolders) && reply.technologyFolders.every(item => typeof item?.token === 'string' && item.token.length > 0
      && (item.databaseIndex == null || templateIndex(item.databaseIndex))
      && (item.folder === null || (typeof item.folder === 'string' && item.folder.length > 0)))
    && (reply.definitions === undefined || (Array.isArray(reply.definitions) && reply.definitions.every(item =>
      typeof item?.token === 'string' && item.token.length > 0 && item.token.length <= 512 && templateIndex(item.databaseIndex)
      && text(item.name, Infinity) && text(item.description, Infinity))));
  if (!valid) throw new Error('Research history does not match this country and recording.');
  keepDrawableIcons(reply.artwork);
  return { ...reply, history: { coverage: reply.history.coverage, slots: reply.history.slots.map(slot => ({ ...slot, states: [], progress: [] })) } };
}

export function validateResearchSlots(input: unknown): ResearchSlotsReply {
  const reply = input as ResearchSlotsReply;
  const valid = !!reply && typeof reply.tag === 'string' && (reply.readHour === null || hour(reply.readHour))
    && text(reply.missing) && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.slots) && reply.slots.every((slot, at) => Number.isInteger(slot.index) && (at === 0 || slot.index > reply.slots[at - 1].index)
      && !!slot.state && validState(slot.state) && (slot.progress === null || validProgress(slot.progress))
      && (isUnjoinedResearch(slot.state)
        ? !!slot.definition && slot.definition.token === slot.state.token && slot.definition.databaseIndex === slot.state.databaseIndex
          && text(slot.definition.name, Infinity) && text(slot.definition.description, Infinity)
        : slot.definition === null))
    && !!reply.grants;
  if (!valid) throw new Error('Research slots do not match this country and recording.');
  return { ...reply, grants: researchGrants(reply.grants) };
}

export function validateResearchBreakdown(input: unknown): ResearchBreakdownReply {
  const reply = input as ResearchBreakdownReply;
  const valid = !!reply && Array.isArray(reply.parts) && reply.parts.every(part => Number.isInteger(part.slot) && hour(part.firstObservedHour)
    && text(part.missing) && intervals(part.states) && part.states.every(validState) && intervals(part.progress) && part.progress.every(validProgress));
  if (!valid) throw new Error('Research breakdown does not match this assignment.');
  return reply;
}
