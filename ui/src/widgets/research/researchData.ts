import { runAt } from '../../charts/series.ts';
import type { Artwork, ResearchBound, ResearchBreakdownReply, ResearchCoverage, ResearchEntry, ResearchFolder, ResearchHistoryReply, ResearchItem, ResearchReply, ResearchSlotNow, ResearchSlotState } from '../../generated/dashboard';
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

export interface ResearchName {
  readonly label: string;
  readonly missing: string | null;
  readonly description: string | null;
}

export interface ResearchLooks {
  name(token: string, databaseIndex: number | null): ResearchName;
  folder(token: string, databaseIndex: number | null): { readonly key: string; readonly name: string } | undefined;
}

const looksCache = new WeakMap<ResearchHistoryReply, ResearchLooks>();
const definitionKey = (token: string, databaseIndex: number | null) => `${token}\u0000${databaseIndex ?? ''}`;

export function researchLooks(reply: ResearchHistoryReply): ResearchLooks {
  const held = looksCache.get(reply);
  if (held) return held;
  const artwork = new Map(reply.artwork.map(art => [art.token, art]));
  const definitions = new Map(reply.definitions.map(item => [definitionKey(item.token, item.databaseIndex), item]));
  const folders = new Map(reply.folders.map((folder: ResearchFolder) => [folder.key, { key: folder.key, name: contentTitle(folder.name, folder.key, 'Folder name unavailable') }]));
  const placed = new Map(reply.technologyFolders.map(item => [definitionKey(item.token, item.databaseIndex ?? null), item.folder]));
  const looks: ResearchLooks = {
    name(token, databaseIndex) {
      const definition = definitions.get(definitionKey(token, databaseIndex));
      const art = artwork.get(token);
      const name = definition?.name ?? art?.name ?? null;
      return { label: name ?? token, missing: name === null ? reply.unnamed : null, description: definition?.description ?? art?.description ?? null };
    },
    folder(token, databaseIndex) {
      const key = placed.get(definitionKey(token, databaseIndex)) ?? placed.get(definitionKey(token, null));
      return key ? folders.get(key) : undefined;
    },
  };
  looksCache.set(reply, looks);
  return looks;
}

export function researchHistoryProof(data: unknown, _requested: readonly [number, number], hour: number): readonly [number, number] | null {
  const reply = data as ResearchHistoryReply;
  const last = reply.nextChange === null ? Number.MAX_SAFE_INTEGER : reply.nextChange - 1;
  return reply.hour <= hour && hour <= last ? [reply.hour, last] : null;
}

const BOUNDS = new Set(['exact', 'by', 'heldBefore', 'lastSeen']);
const OUTCOMES = new Set(['active', 'completed', 'switched', 'unknown']);
const validBound = (bound: ResearchBound | null) => bound === null || (!!bound && BOUNDS.has(bound.kind) && hour(bound.hour));
const validItem = (item: ResearchItem, through: number) => typeof item?.token === 'string' && item.token.length > 0 && item.token.length <= 512
  && templateIndex(item.databaseIndex) && hour(item.firstObservedHour) && item.firstObservedHour <= through && OUTCOMES.has(item.outcome)
  && intervals(item.pieces) && item.pieces.every(piece => piece.lastHour <= through)
  && validBound(item.start) && validBound(item.end) && (item.end === null) === (item.outcome === 'active')
  && (item.origin === null || (Number.isInteger(item.origin.slot) && validBound(item.origin.start)))
  && (item.tookDays === null || (Number.isInteger(item.tookDays) && item.tookDays >= 0));

export function validateResearchHistory(input: unknown): ResearchHistoryReply {
  const reply = input as ResearchHistoryReply;
  let last = -1;
  const valid = !!reply && typeof reply === 'object' && hour(reply.hour) && (reply.nextChange === null || (hour(reply.nextChange) && reply.nextChange > reply.hour))
    && text(reply.unrecorded) && text(reply.partial) && typeof reply.unnamed === 'string' && Number.isInteger(reply.completed) && reply.completed >= 0
    && intervals(reply.coverage) && reply.coverage.every(span => span.lastHour <= reply.hour)
    && Array.isArray(reply.slots) && reply.slots.every(slot => {
      const ordered = Number.isInteger(slot?.index) && slot.index > last;
      last = slot?.index;
      return ordered && intervals(slot.availability) && slot.availability.every(span => span.lastHour <= reply.hour)
        && Array.isArray(slot.items) && slot.items.every(item => validItem(item, reply.hour));
    })
    && Array.isArray(reply.artwork) && reply.artwork.every(artworkEntry)
    && Array.isArray(reply.folders) && reply.folders.every(folder => typeof folder?.key === 'string' && folder.key.length > 0 && text(folder.name))
    && Array.isArray(reply.technologyFolders) && reply.technologyFolders.every(item => typeof item?.token === 'string' && item.token.length > 0
      && (item.databaseIndex == null || templateIndex(item.databaseIndex)) && (item.folder === null || (typeof item.folder === 'string' && item.folder.length > 0)))
    && Array.isArray(reply.definitions) && reply.definitions.every(item => typeof item?.token === 'string' && item.token.length > 0
      && templateIndex(item.databaseIndex) && text(item.name, Infinity) && text(item.description, Infinity))
    && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string');
  if (!valid) throw new Error('Research history does not match this country and recording.');
  keepDrawableIcons(reply.artwork);
  return reply;
}
