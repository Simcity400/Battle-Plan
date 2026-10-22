import { runAt } from '../../charts/series.ts';
import type { ProductRequest } from '../../dashboard/client';
import type { Artwork, CountryParams, ResearchReply } from '../../generated/dashboard';
import { artworkEntry, keepDrawableIcons } from '../../dashboard/artwork.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';

export const PRODUCT = 'research';

export function researchRequest(sourceId: string, revision: string, params: CountryParams): ProductRequest<CountryParams> {
  return { sourceId, revision, product: PRODUCT, params };
}
import type { ResearchHistory, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';

/** Playback never exposes future outcomes, assignments, or slot additions.
 * History up to `hour` stays known when that hour itself has no read: only
 * the slot states looked up at the hour are then missing. */
export function researchAt(history: ResearchHistory, hour: number): ResearchSlotHistory[] {
  return history.slots.filter(s => s.availability[0]?.firstHour <= hour).map(slot => ({
    ...slot,
    availability: slot.availability.filter(c => c.firstHour <= hour).map(c => ({ ...c, lastHour: Math.min(c.lastHour, hour) })),
    states: (slot.states ?? []).filter(s => s.firstHour <= hour).map(s => ({ ...s, lastHour: Math.min(s.lastHour, hour) })),
    progress: (slot.progress ?? []).filter(p => p.firstHour <= hour).map(p => ({ ...p, lastHour: Math.min(p.lastHour, hour) })),
    entries: slot.entries.filter(e => e.firstObservedHour <= hour).map(e => ({ ...e,
      lastObservedHour: Math.min(e.lastObservedHour, hour),
      endedByHour: e.endedByHour !== null && e.endedByHour <= hour ? e.endedByHour : null,
      outcome: e.endedByHour !== null && e.endedByHour <= hour
        ? e.outcomeObservedHour != null && e.outcomeObservedHour > hour ? 'unknown' : e.outcome : 'active',
    })),
  }));
}

/** Resolve one assignment without visiting the slot's full history. */
export function researchEntryAt(slot: ResearchSlotHistory, state: ResearchSlotState, hour: number) {
  let low = 0, high = slot.entries.length;
  while (low < high) { const mid = (low + high) >>> 1; if (slot.entries[mid].firstObservedHour < (state.entryFirstObservedHour ?? -Infinity)) low = mid + 1; else high = mid; }
  for (; low < slot.entries.length && slot.entries[low].firstObservedHour === state.entryFirstObservedHour; low++) {
    const entry = slot.entries[low];
    if (entry.token === state.token && entry.firstObservedHour <= hour && (entry.endedByHour === null || entry.endedByHour > hour))
      return { ...entry, lastObservedHour: Math.min(entry.lastObservedHour, hour), endedByHour: null, outcome: 'active' as const };
  }
  return null;
}

/** A slot state naming a technology that no assignment joins: the recording
 * did not identify which occurrence of it the slot held. It is shown, but its
 * progress and completion stay unknown. */
export function isUnjoinedResearch(state: ResearchSlotState | null): state is ResearchSlotState & { token: string } {
  return !!state?.token && state.entryFirstObservedHour === null;
}

export function researchStateAt(slot: ResearchSlotHistory, hour: number): ResearchSlotState | null {
  if (Number.isNaN(hour)) return null;
  if (!runAt(slot.availability, hour)) return null;
  const state = runAt(slot.states ?? [], hour);
  if (!state) return null;
  return state.details?.remainingDays == null ? state : { ...state, details: { ...state.details,
    remainingDays: Math.max(0, state.details.remainingDays - Math.floor((hour - state.firstHour) / 24)),
  } };
}

/** Recorded progress toward an assignment's current base cost at `hour`, as a
 * 0-1 fraction. Null without a sample at that hour or a known positive cost;
 * never carried across hours that were not recorded. */
export function researchProgressAt(slot: ResearchSlotHistory, entryFirstObservedHour: number, hour: number): number | null {
  if (Number.isNaN(hour)) return null;
  const sample = runAt(slot.progress ?? [], hour);
  if (!sample?.baseDaysRaw || sample.entryFirstObservedHour !== entryFirstObservedHour) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  if (base <= 0n || done < 0n) return null;
  return done >= base ? 1 : Number(done * 10_000n / base) / 10_000;
}

/** Integer research rate with the selected hour's ahead-of-time penalty. */
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

/** A run of reads in which a slot held one technology without its recorded
 * occurrence (older recordings). `endedByHour` is the read at which it had left
 * the slot; null when the run ends at a gap or the slot closing, where the
 * recording does not say. */
export interface ResearchHeldRun { token: string; firstHour: number; lastHour: number; endedByHour: number | null }
const heldIndexes = new WeakMap<ResearchSlotHistory, ResearchHeldRun[]>();

/** Consecutive slot states holding the same technology without a joined
 * assignment, one run each. Never joined across a break in the slot's states. */
export function researchHeldRuns(slot: ResearchSlotHistory): ResearchHeldRun[] {
  const indexed = heldIndexes.get(slot);
  if (indexed) return indexed;
  const runs: ResearchHeldRun[] = [];
  let open: ResearchHeldRun | null = null;
  for (const state of slot.states ?? []) {
    const contiguous = open !== null && open.lastHour + 1 === state.firstHour;
    if (open && contiguous && isUnjoinedResearch(state) && state.token === open.token) { open.lastHour = state.lastHour; continue; }
    if (open) { if (contiguous) open.endedByHour = state.firstHour; runs.push(open); open = null; }
    if (isUnjoinedResearch(state)) open = { token: state.token, firstHour: state.firstHour, lastHour: state.lastHour, endedByHour: null };
  }
  if (open) runs.push(open);
  heldIndexes.set(slot, runs);
  return runs;
}

const progressIndexes = new WeakMap<ResearchSlotHistory, Map<number | null, ResearchSlotHistory['progress']>>();

/** The latest recorded progress of what `state` holds, at or before `hour`, with
 * the hour of the read it came from: the assignment's own samples, or for a
 * technology held without its recorded occurrence, the samples since the slot
 * has held it without a break. Never interpolated or taken from another
 * technology; null without a sample or a known positive cost. */
export function researchLatestProgress(slot: ResearchSlotHistory, state: ResearchSlotState, hour: number): { fraction: number; hour: number } | null {
  let since = -Infinity;
  if (state.entryFirstObservedHour === null) {
    if (!state.token) return null;
    const run = runAt(researchHeldRuns(slot), hour);
    if (run?.token !== state.token) return null;
    if (!run) return null;
    since = run.firstHour;
  }
  let index = progressIndexes.get(slot);
  if (!index) {
    index = new Map();
    for (const sample of slot.progress ?? []) {
      const samples = index.get(sample.entryFirstObservedHour) ?? [];
      samples.push(sample); index.set(sample.entryFirstObservedHour, samples);
    }
    progressIndexes.set(slot, index);
  }
  const samples = index.get(state.entryFirstObservedHour) ?? [];
  let low = 0, high = samples.length;
  while (low < high) { const mid = (low + high) >>> 1; if (samples[mid].firstHour <= hour) low = mid + 1; else high = mid; }
  const sample = samples[low - 1];
  if (sample && sample.lastHour < since) return null;
  if (!sample?.baseDaysRaw) return null;
  const base = BigInt(sample.baseDaysRaw), done = BigInt(sample.progressRaw);
  if (base <= 0n || done < 0n) return null;
  return { fraction: done >= base ? 1 : Number(done * 10_000n / base) / 10_000, hour: Math.min(sample.lastHour, hour) };
}

/** A technology's research folder, named and coloured; `order` is its place
 * among the folders shown, in content order. */
export interface ResearchFolderLook { key: string; name: string; colour: string; order: number }

/** Keep token and index separate: quoted technology tokens can contain any character. */
export type ResearchUse = string | readonly [token: string, databaseIndex: number];

/** Separate token and definition lookups to avoid collisions. */
export type ResearchFolderLooks = ReadonlyMap<string, ResearchFolderLook>
  & { readonly definitions?: ReadonlyMap<string, ReadonlyMap<number, ResearchFolderLook>> };

/** Use the whole history to keep folder colours stable through playback. */
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

/** Separate token and definition text lookups to avoid collisions. */
export type ResearchArtwork = Map<string, Artwork> & { readonly definitions: ReadonlyMap<string, ReadonlyMap<number, Artwork>> };

export function researchArtwork(reply: Pick<ResearchReply, 'artwork' | 'definitions'> | null | undefined,
  images: ReadonlyMap<string, Artwork>): ResearchArtwork {
  const artwork = new Map<string, Artwork>(), definitions = new Map<string, Map<number, Artwork>>();
  for (const art of reply?.artwork ?? []) artwork.set(art.token, { ...art, icon: images.get(art.token)?.icon ?? art.icon });
  for (const definition of reply?.definitions ?? []) {
    const art = artwork.get(definition.token);
    definitions.set(definition.token, (definitions.get(definition.token) ?? new Map<number, Artwork>()).set(definition.databaseIndex,
      { token: definition.token, name: definition.name, description: definition.description, baseDurationDays: art?.baseDurationDays ?? null,
        icon: art?.icon ?? images.get(definition.token)?.icon ?? null }));
  }
  return Object.assign(artwork, { definitions });
}

/** Null marks an unassigned use; tokens with no recorded definition are omitted. */
export function researchRecordedTemplates(history: ResearchHistory | null | undefined): Map<string, (number | null)[]> {
  const recorded = new Map<string, Set<number | null>>();
  const hold = (token: string, index: number | null) => recorded.set(token, (recorded.get(token) ?? new Set<number | null>()).add(index));
  for (const slot of history?.slots ?? []) {
    for (const entry of slot.entries) hold(entry.token, entry.databaseIndex ?? null);
    for (const state of slot.states ?? []) if (state.token != null && state.entryFirstObservedHour == null) hold(state.token, null);
  }
  return new Map([...recorded].flatMap(([token, uses]) => [...uses].some(index => index != null)
    ? [[token, [...uses].sort((a, b) => (a ?? 0) - (b ?? 0))] as const] : []));
}

/** Without a recorded index, conflicting technology definitions must stay refused by name. */
export function researchArt(artwork: ResearchArtwork, token: string, databaseIndex?: number | null): Artwork | undefined {
  return (databaseIndex == null ? undefined : artwork.definitions.get(token)?.get(databaseIndex)) ?? artwork.get(token);
}

/** Includes recorded definitions and preserves content order. */
export function researchFolderList(looks: ResearchFolderLooks): ResearchFolderLook[] {
  const seen = new Map<string, ResearchFolderLook>();
  const all = [...looks.values(), ...[...looks.definitions?.values() ?? []].flatMap(byIndex => [...byIndex.values()])];
  for (const look of all) if (!seen.has(look.key)) seen.set(look.key, look);
  return [...seen.values()].sort((a, b) => a.order - b.order);
}

/** Images for the cards displayed at this hour: current assignments first,
 * then completed rows in column order. Repeated technologies share one image. */
export function researchArtworkTokens(slots: readonly ResearchSlotHistory[] | null, hour: number | undefined, moment: boolean): string[] {
  if (!slots || hour === undefined) return [];
  const ordered = [...slots].sort((a, b) => a.index - b.index), tokens: string[] = [];
  if (moment) for (const slot of ordered) {
    const current = researchStateAt(slot, hour);
    const entry = current && slot.entries.find(entry => entry.outcome === 'active'
      && entry.firstObservedHour === current.entryFirstObservedHour && entry.token === current.token);
    if (entry) tokens.push(entry.token);
    else if (isUnjoinedResearch(current)) tokens.push(current.token);
  }
  for (const slot of ordered) {
    for (const entry of slot.entries) if (entry.outcome === 'completed') tokens.push(entry.token);
    // A technology still held at this hour is not yet history.
    for (const run of researchHeldRuns(slot)) if (run.lastHour < hour) tokens.push(run.token);
  }
  return [...new Set(tokens)];
}

/** Shape checks on the product payload; identity is checked by the transport. */
export function validateResearchReply(input: unknown): ResearchReply {
  const reply = input as ResearchReply;
  const hour = (n: number) => Number.isInteger(n) && n >= -2147483648 && n <= 2147483647;
  // Any number of rows: a long modded campaign is bounded only by the reply's own budget.
  const intervals = (spans: ResearchHistory['coverage']) => {
    let last = -Infinity;
    return Array.isArray(spans) && spans.every(s => {
      const valid = hour(s.firstHour) && hour(s.lastHour) && s.firstHour > last && s.lastHour >= s.firstHour;
      last = s.lastHour;
      return valid;
    });
  };
  const fixed = (value: unknown): value is string => typeof value === 'string' && /^-?\d{1,19}$/.test(value)
    && BigInt(value) >= -9223372036854775808n && BigInt(value) <= 9223372036854775807n;
  // A recorded template index: one past its place in the technology database, after the null template.
  const templateIndex = (value: unknown): value is number => Number.isInteger(value) && (value as number) >= 1 && (value as number) <= 4294967295;
  let index = -1;
  const valid = !!reply && typeof reply === 'object' && Array.isArray(reply.artwork) && Array.isArray(reply.history?.slots)
    // Any number of slots: content and effects set how many a country has.
    && intervals(reply.history.coverage) && reply.history.slots.every(s => {
      let last = -Infinity;
      const valid = Number.isInteger(s.index) && s.index > index && s.availability.length > 0 && intervals(s.availability)
        && s.entries.every(e => {
          const ok = typeof e.token === 'string' && e.token.length > 0 && e.token.length <= 512 && Number.isInteger(e.level)
            && Number.isInteger(e.technologyHandle) && e.technologyHandle >= 0 && e.technologyHandle <= 4294967295
            && (e.databaseIndex == null || templateIndex(e.databaseIndex))
            && hour(e.continuityStartHour) && e.continuityStartHour <= e.firstObservedHour
            && hour(e.firstObservedHour) && e.firstObservedHour >= last && hour(e.lastObservedHour) && e.lastObservedHour >= e.firstObservedHour
            && (e.startedAfterHour === null || (hour(e.startedAfterHour) && e.startedAfterHour < e.firstObservedHour))
            && (e.outcomeObservedHour == null || (hour(e.outcomeObservedHour) && e.endedByHour !== null && e.outcomeObservedHour >= e.endedByHour))
            && ['active', 'completed', 'switched', 'unknown'].includes(e.outcome)
            && (e.endedByHour === null ? e.outcome === 'active' : hour(e.endedByHour) && e.endedByHour >= e.lastObservedHour && e.outcome !== 'active');
          last = e.endedByHour ?? Infinity;
          return ok;
        }) && intervals(s.progress) && s.progress.every(sample => {
          // A sample without an assignment belongs to the technology the slot
          // held without its recorded occurrence; that hour's state names it.
          return (sample.entryFirstObservedHour === null
            ? Array.isArray(s.states) && s.states.some(state => state.entryFirstObservedHour === null && typeof state.token === 'string'
              && state.firstHour <= sample.firstHour && sample.firstHour <= state.lastHour)
            : s.entries.some(e => e.firstObservedHour === sample.entryFirstObservedHour
              && sample.firstHour >= e.firstObservedHour
              && sample.lastHour <= (e.endedByHour ?? e.lastObservedHour)))
            && s.availability.some(span => span.firstHour <= sample.firstHour && sample.lastHour <= span.lastHour)
            && fixed(sample.progressRaw) && (sample.baseDaysRaw === null || (fixed(sample.baseDaysRaw) && BigInt(sample.baseDaysRaw) > 0n));
        }) && (s.states === undefined || (intervals(s.states) && s.states.every(state => {
          return (state.token === null || (typeof state.token === 'string' && state.token.length > 0 && state.token.length <= 512))
            && (state.entryFirstObservedHour === null || s.entries.some(e => e.firstObservedHour === state.entryFirstObservedHour && e.token === state.token))
            && s.availability.some(span => span.firstHour <= state.firstHour && state.lastHour <= span.lastHour)
            && fixed(state.speedMultiplierRaw) && fixed(state.bankedDaysRaw) && fixed(state.appliedDaysRaw)
            && (state.aheadReductionYearsRaw === null || fixed(state.aheadReductionYearsRaw))
            && (state.details == null || (
              (state.details.remainingDays === null || (Number.isInteger(state.details.remainingDays) && state.details.remainingDays >= 0 && state.details.remainingDays <= 4294967295))
              && (state.details.modifiers == null || (hour(state.details.modifiers.year)
                && fixed(state.details.modifiers.totalBonusRaw) && fixed(state.details.modifiers.aheadFactorRaw) && fixed(state.details.modifiers.minimumSpeedRaw)
                && Array.isArray(state.details.modifiers.components) && state.details.modifiers.components.length === 8
                && new Set(state.details.modifiers.components.map(c => c.kind)).size === 8
                && state.details.modifiers.components.every(c => ['country', 'category', 'designTeam', 'experience', 'license', 'scientist', 'applied', 'sharing'].includes(c.kind)
                  && fixed(c.raw) && Array.isArray(c.sources) && c.sources.every(source => fixed(source.raw)
                    && (source.token === null || (typeof source.token === 'string' && source.token.length <= 512))
                    && (source.name === null || (typeof source.name === 'string' && source.name.length <= 4096))))))
              && (state.details.bonuses === null || (Array.isArray(state.details.bonuses)
                && state.details.bonuses.every(b => Number.isInteger(b.id) && b.id >= -2147483648 && b.id <= 2147483647
                  && (b.token === null || (typeof b.token === 'string' && b.token.length <= 512))
                  && (b.name === null || (typeof b.name === 'string' && b.name.length <= 4096))
                  && (b.speedBonusRaw === null || fixed(b.speedBonusRaw))
                  && (b.costReductionRaw == null || fixed(b.costReductionRaw))
                  && (b.aheadReductionYearsRaw === null || fixed(b.aheadReductionYearsRaw)))))));
        })));
      index = s.index;
      return valid;
    // Text-only history titles cover every technology reached by the history.
    // Each entry must have the product's shape; only an icon it cannot draw becomes missing art.
    }) && reply.artwork.every(artworkEntry)
    && Array.isArray(reply.folders) && reply.folders.every(folder => typeof folder?.key === 'string' && folder.key.length > 0
      && (folder.name === null || typeof folder.name === 'string'))
    && Array.isArray(reply.technologyFolders) && reply.technologyFolders.every(item => typeof item?.token === 'string' && item.token.length > 0
      && (item.databaseIndex == null || templateIndex(item.databaseIndex))
      && (item.folder === null || (typeof item.folder === 'string' && item.folder.length > 0)))
    && (reply.definitions === undefined || (Array.isArray(reply.definitions) && reply.definitions.every(item =>
      typeof item?.token === 'string' && item.token.length > 0 && item.token.length <= 512 && templateIndex(item.databaseIndex)
      && (item.name === null || typeof item.name === 'string') && (item.description === null || typeof item.description === 'string'))));
  if (!valid) throw new Error('Research history does not match this country and recording.');
  keepDrawableIcons(reply.artwork);
  return reply;
}
