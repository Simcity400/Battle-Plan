import { classifyFailure, RefusedError, type Failure } from '../../dashboard/queryError.ts';
import type {
  CountryFactsReply,
  CountryFlagChoice,
  CountryFlagSize,
  CountryGroupReply,
  CountryIdentitiesReply,
  CountryStyle,
  CountryStyleReply,
  FactionGroup,
  HourSpan,
} from '../../generated/dashboard';
import { compareTags } from '../../dashboard/tagOrder.ts';
import { artHandleKey } from '../art/types.ts';
import { resolveSlotHolderRuns, type HolderRun } from '../holders.ts';
import {
  countryTagKey,
  flagChoiceKey,
  identityKeyString,
  styleEntryFromReply,
  type CountryHolder,
  type CountryIdentityKey,
  type CountryIdentityResult,
  type DrawnFlagEntry,
  type EntityIdentityKey,
  type EntityIdentityResult,
  type EntityRunEntry,
  type FactionIdentityKey,
  type FactionIdentityResult,
  type HourInterval,
  type IdentityField,
  type IdentityKey,
  type IdentityResult,
  type IdentityScope,
  type IdentitySnapshot,
  type SnapshotFailure,
  type SnapshotFailures,
  type StyleIntervalEntry,
} from './types.ts';

type FactRun = readonly [number, number, number, number, number];
type RawSlotEntry = { readonly firstHour: number; readonly lastHour: number; readonly countryId: number; readonly tag: string };

const NOT_A_COUNTRY = 'not a country at this hour';
const FACTS_NOT_RECORDED = 'country facts were not recorded at this hour';
const NO_COLOUR = 'no recorded map colour';
const NO_NAME = 'Country name unavailable';
const NO_FLAG = 'flag unavailable';
const FACTIONS_NOT_RECORDED = 'Factions were not recorded at this hour';
const FACTION_ABSENT = 'Faction does not exist at this hour';

export function drawnFlagKey(size: CountryFlagSize, choice: CountryFlagChoice): string {
  return artHandleKey({ kind: 'flag', choice: flagChoiceKey(choice), size });
}

function sentFailure(failure: Failure): SnapshotFailure {
  return { message: failure.message, refused: classifyFailure(failure)?.kind === 'refused' };
}

function keptFailure(sent: SnapshotFailure): Failure {
  return sent.refused ? new RefusedError(sent.message) : new Error(sent.message);
}

function sentFailures(failures: ReadonlyMap<string, Failure>): Record<string, SnapshotFailure> | undefined {
  if (!failures.size) return undefined;
  const out: Record<string, SnapshotFailure> = {};
  for (const [key, failure] of failures) out[key] = sentFailure(failure);
  return out;
}

function nextBoundaryOf(lastHour: number): number {
  return Number.isFinite(lastHour) ? lastHour + 1 : Infinity;
}

function intersectIntervals(a: HourInterval, b: HourInterval): HourInterval {
  const first = Math.max(a[0], b[0]);
  const last = Math.min(a[1], b[1]);
  return first <= last ? [first, last] : [first, first];
}

function unpackRgb(packed: number): readonly [number, number, number] {
  return [(packed >> 16) & 255, (packed >> 8) & 255, packed & 255];
}

function findSpanIndex(spans: readonly (readonly [number, number])[], hour: number): number {
  let low = 0;
  let high = spans.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (spans[mid]![1] < hour) low = mid + 1;
    else high = mid;
  }
  return low;
}

function findHourSpanIndex(spans: readonly HourSpan[], hour: number): number {
  let low = 0;
  let high = spans.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (spans[mid]!.lastHour < hour) low = mid + 1;
    else high = mid;
  }
  return low;
}

function enclosingGap(
  spans: readonly HourSpan[],
  hour: number,
  bounds: HourInterval,
): { readonly inside: HourSpan | null; readonly interval: HourInterval } {
  if (!spans.length) return { inside: null, interval: bounds };
  const idx = findHourSpanIndex(spans, hour);
  const candidate = spans[idx];
  if (candidate && candidate.firstHour <= hour && hour <= candidate.lastHour) {
    return {
      inside: candidate,
      interval: [Math.max(bounds[0], candidate.firstHour), Math.min(bounds[1], candidate.lastHour)],
    };
  }
  const prev = idx > 0 ? spans[idx - 1] : undefined;
  const first = prev ? Math.max(bounds[0], prev.lastHour + 1) : bounds[0];
  const last = candidate ? Math.min(bounds[1], candidate.firstHour - 1) : bounds[1];
  return { inside: null, interval: [first, last] };
}

function sortedHourSpans(spans: readonly HourSpan[] | undefined): readonly HourSpan[] {
  if (!spans?.length) return [];
  return [...spans].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
}

function findRunBeforeOrAt<T>(runs: readonly T[], hour: number, firstOf: (item: T) => number): number {
  let low = 0;
  let high = runs.length;
  while (low < high) {
    const mid = (low + high) >> 1;
    if (firstOf(runs[mid]!) <= hour) low = mid + 1;
    else high = mid;
  }
  return low - 1;
}

function mergeStyleEntry(list: StyleIntervalEntry[], next: StyleIntervalEntry): StyleIntervalEntry[] {
  const kept = list.filter(item => item.lastHour < next.firstHour || item.firstHour > next.lastHour);
  kept.push(next);
  kept.sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  return kept;
}

function findCoveringStyle(runs: readonly StyleIntervalEntry[] | undefined, hour: number): StyleIntervalEntry | null {
  if (!runs?.length) return null;
  const idx = findRunBeforeOrAt(runs, hour, item => item.firstHour);
  const candidate = idx >= 0 ? runs[idx] : undefined;
  return candidate && candidate.firstHour <= hour && hour <= candidate.lastHour ? candidate : null;
}

function findCoveringEntity(runs: readonly EntityRunEntry[] | undefined, hour: number): EntityRunEntry | null {
  if (!runs?.length) return null;
  const idx = findRunBeforeOrAt(runs, hour, item => item.firstHour);
  const candidate = idx >= 0 ? runs[idx] : undefined;
  return candidate && candidate.firstHour <= hour && hour <= candidate.lastHour ? candidate : null;
}

export class IdentityIndex {
  readonly scope: IdentityScope;
  private branchBounds: HourInterval;
  private facts?: CountryFactsReply;
  private factsFinal = false;
  private factsByTag = new Map<string, readonly FactRun[]>();
  private factsKnown: readonly (readonly [number, number])[] = [];
  private factsFailure?: Failure;

  private identities?: CountryIdentitiesReply;
  private identitiesKnown: readonly HourSpan[] | null = null;
  private identitiesUnknown: readonly HourSpan[] = [];
  private slotRuns = new Map<number, readonly HolderRun[]>();
  private tagIdentities = new Map<string, readonly { readonly firstHour: number; readonly lastHour: number; readonly slot: number; readonly countryId: number }[]>();
  private tagUnresolved = new Map<string, readonly HourSpan[]>();
  private identitiesFailure?: Failure;

  private groups?: CountryGroupReply;
  private factionsById = new Map<string, FactionGroup>();
  private factionsUnobserved: readonly HourSpan[] = [];
  private factionIncomplete = new Map<string, readonly HourSpan[]>();
  private groupsFailure?: Failure;

  private styleRuns = new Map<string, StyleIntervalEntry[]>();
  private styleFailures = new Map<string, Failure>();
  private entityRuns = new Map<string, EntityRunEntry[]>();
  private entityFailures = new Map<string, Failure>();
  private drawnFlags = new Map<string, DrawnFlagEntry>();

  constructor(scope: IdentityScope, branchBounds?: HourInterval) {
    this.scope = scope;
    this.branchBounds = branchBounds ?? [-Infinity, Infinity];
  }

  static fromSnapshot(snapshot: IdentitySnapshot): IdentityIndex {
    const index = new IdentityIndex(snapshot.scope, snapshot.branchBounds);
    if (snapshot.facts) index.setFacts(snapshot.facts, snapshot.factsFinal ?? true);
    if (snapshot.identities) index.setIdentities(snapshot.identities);
    if (snapshot.groups) index.setGroups(snapshot.groups);
    if (snapshot.styles) {
      for (const [tag, entries] of Object.entries(snapshot.styles)) {
        index.styleRuns.set(tag, [...entries].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour));
      }
    }
    if (snapshot.entities) {
      for (const [key, entries] of Object.entries(snapshot.entities)) {
        index.entityRuns.set(key, [...entries].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour));
      }
    }
    if (snapshot.drawnFlags) {
      for (const [key, entry] of Object.entries(snapshot.drawnFlags)) {
        index.drawnFlags.set(key, entry);
      }
    }
    const failures = snapshot.failures;
    if (failures?.facts) index.factsFailure = keptFailure(failures.facts);
    if (failures?.identities) index.identitiesFailure = keptFailure(failures.identities);
    if (failures?.groups) index.groupsFailure = keptFailure(failures.groups);
    for (const [key, sent] of Object.entries(failures?.styles ?? {})) index.styleFailures.set(key, keptFailure(sent));
    for (const [key, sent] of Object.entries(failures?.entities ?? {})) index.entityFailures.set(key, keptFailure(sent));
    return index;
  }

  exportSnapshot(): IdentitySnapshot {
    const styles: Record<string, readonly StyleIntervalEntry[]> = {};
    for (const [tag, entries] of this.styleRuns) styles[tag] = entries;
    const entities: Record<string, readonly EntityRunEntry[]> = {};
    for (const [key, entries] of this.entityRuns) entities[key] = entries;
    const drawnFlags: Record<string, DrawnFlagEntry> = {};
    for (const [key, entry] of this.drawnFlags) drawnFlags[key] = entry;
    const styleFailures = sentFailures(this.styleFailures);
    const entityFailures = sentFailures(this.entityFailures);
    const failures: SnapshotFailures = {
      ...(this.factsFailure ? { facts: sentFailure(this.factsFailure) } : {}),
      ...(this.identitiesFailure ? { identities: sentFailure(this.identitiesFailure) } : {}),
      ...(this.groupsFailure ? { groups: sentFailure(this.groupsFailure) } : {}),
      ...(styleFailures ? { styles: styleFailures } : {}),
      ...(entityFailures ? { entities: entityFailures } : {}),
    };
    return {
      scope: this.scope,
      ...(Number.isFinite(this.branchBounds[0]) && Number.isFinite(this.branchBounds[1]) ? { branchBounds: this.branchBounds } : {}),
      ...(this.facts ? { facts: this.facts, factsFinal: this.factsFinal } : {}),
      ...(this.identities ? { identities: this.identities } : {}),
      ...(this.groups ? { groups: this.groups } : {}),
      ...(this.styleRuns.size ? { styles } : {}),
      ...(this.entityRuns.size ? { entities } : {}),
      ...(this.drawnFlags.size ? { drawnFlags } : {}),
      ...(Object.keys(failures).length ? { failures } : {}),
    };
  }

  setBranchBounds(bounds: HourInterval): void {
    this.branchBounds = bounds;
  }

  bounds(): HourInterval {
    return this.branchBounds;
  }

  setFacts(reply: CountryFactsReply, final = true): void {
    this.facts = reply;
    this.factsFinal = final;
    this.factsFailure = undefined;
    this.factsKnown = reply.known.map(span => [span[0], span[1]] as const);
    this.factsByTag = new Map(reply.countries.map(country => [country.tag, country.runs as readonly FactRun[]]));
  }

  setFactsFailure(failure: Failure): void {
    this.factsFailure = failure;
  }

  clearFactsFailure(): void {
    this.factsFailure = undefined;
  }

  hasFactsAt(hour: number): boolean {
    if (!this.facts) return false;
    if (this.factsFinal) return true;
    const at = Math.floor(hour);
    const idx = findSpanIndex(this.factsKnown, at);
    return idx < this.factsKnown.length && this.factsKnown[idx]![0] <= at;
  }

  hasFacts(): boolean {
    return this.facts !== undefined;
  }

  setIdentities(reply: CountryIdentitiesReply): void {
    this.identities = reply;
    this.identitiesFailure = undefined;
    this.identitiesKnown = reply.known?.length ? sortedHourSpans(reply.known) : null;
    this.identitiesUnknown = sortedHourSpans(reply.unknown.map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour })));
    this.tagIdentities.clear();
    this.tagUnresolved.clear();
    const bySlot = new Map<number, RawSlotEntry[]>();
    for (const country of reply.countries) {
      const sorted = [...country.identities].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
      this.tagIdentities.set(country.tag, sorted);
      if (country.unresolved.length) {
        this.tagUnresolved.set(country.tag, sortedHourSpans(country.unresolved));
      }
      for (const run of sorted) {
        const list = bySlot.get(run.slot);
        const entry: RawSlotEntry = { firstHour: run.firstHour, lastHour: run.lastHour, countryId: run.countryId, tag: country.tag };
        if (list) list.push(entry);
        else bySlot.set(run.slot, [entry]);
      }
    }
    this.slotRuns.clear();
    for (const [slot, list] of bySlot) {
      list.sort((a, b) => a.firstHour - b.firstHour || a.countryId - b.countryId || compareTags(a.tag, b.tag));
      this.slotRuns.set(slot, resolveSlotHolderRuns(list));
    }
  }

  setIdentitiesFailure(failure: Failure): void {
    this.identitiesFailure = failure;
  }

  hasIdentities(): boolean {
    return this.identities !== undefined;
  }

  setGroups(reply: CountryGroupReply): void {
    this.groups = reply;
    this.groupsFailure = undefined;
    this.factionsById = new Map(reply.factions.map(faction => [faction.id, faction]));
    this.factionsUnobserved = sortedHourSpans(reply.unobserved.factions);
    this.factionIncomplete = new Map(reply.factions.map(faction => [faction.id, sortedHourSpans(faction.incomplete)]));
  }

  setGroupsFailure(failure: Failure): void {
    this.groupsFailure = failure;
  }

  hasGroups(): boolean {
    return this.groups !== undefined;
  }

  ingestStylesReply(reply: CountryStyleReply): void {
    for (const style of reply.styles) {
      this.ingestStyle(style, reply.hour, reply.warnings);
    }
  }

  ingestStyle(style: CountryStyle, hour: number, warnings: readonly string[] = []): void {
    const entry = styleEntryFromReply(style, Math.floor(hour), warnings);
    const existing = this.styleRuns.get(style.tag) ?? [];
    this.styleRuns.set(style.tag, mergeStyleEntry(existing, entry));
    this.styleFailures.delete(`${Math.floor(hour)}\u0000${style.tag}`);
  }

  setStyleFailure(tag: string, hour: number, failure: Failure): void {
    this.styleFailures.set(`${Math.floor(hour)}\u0000${tag}`, failure);
  }

  clearStyleFailure(tag: string, hour: number): void {
    this.styleFailures.delete(`${Math.floor(hour)}\u0000${tag}`);
  }

  setEntityRuns(key: EntityIdentityKey, runs: readonly EntityRunEntry[]): void {
    const id = identityKeyString(key);
    this.entityRuns.set(id, [...runs].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour));
    this.entityFailures.delete(id);
  }

  addEntityRun(key: EntityIdentityKey, run: EntityRunEntry): void {
    const id = identityKeyString(key);
    const kept = (this.entityRuns.get(id) ?? []).filter(item => item.lastHour < run.firstHour || item.firstHour > run.lastHour);
    kept.push(run);
    kept.sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
    this.entityRuns.set(id, kept);
    this.entityFailures.delete(id);
  }

  setEntityFailure(key: EntityIdentityKey, failure: Failure): void {
    this.entityFailures.set(identityKeyString(key), failure);
  }

  clearEntityFailures(): void {
    this.entityFailures.clear();
  }

  setDrawnFlag(size: CountryFlagSize, choice: CountryFlagChoice, entry: DrawnFlagEntry): void {
    this.drawnFlags.set(drawnFlagKey(size, choice), entry);
  }

  getDrawnFlag(size: CountryFlagSize, choice: CountryFlagChoice): DrawnFlagEntry | undefined {
    return this.drawnFlags.get(drawnFlagKey(size, choice));
  }

  forgetDrawnFlag(size: CountryFlagSize, choice: CountryFlagChoice): void {
    this.drawnFlags.delete(drawnFlagKey(size, choice));
  }

  slotHolderAt(slot: number, hour: number): { readonly tag: string; readonly countryId: number; readonly valid: HourInterval } | null {
    const at = Math.floor(hour);
    let bounds: HourInterval | null = null;
    if (this.identitiesKnown) {
      const known = enclosingGap(this.identitiesKnown, at, this.branchBounds);
      if (!known.inside) return null;
      bounds = known.interval;
    }
    const runs = this.slotRuns.get(slot);
    if (!runs?.length) return null;
    const idx = findRunBeforeOrAt(runs, at, run => run.firstHour);
    const run = idx >= 0 ? runs[idx] : undefined;
    if (!run || run.firstHour > at || at > run.lastHour) return null;
    const valid: HourInterval = bounds ? intersectIntervals([run.firstHour, run.lastHour], bounds) : [run.firstHour, run.lastHour];
    return { tag: run.tag, countryId: run.countryId ?? 0, valid };
  }

  resolve<K extends IdentityKey>(key: K, hour: number): IdentityResult<K> {
    const at = Math.floor(hour);
    if (!Number.isFinite(at)) {
      return { kind: 'absent', valid: [at, at], nextBoundary: Infinity, reason: 'Invalid hour' } as unknown as IdentityResult<K>;
    }
    if (key.kind === 'country') return this.resolveCountry(key, at) as IdentityResult<K>;
    if (key.kind === 'faction') return this.resolveFaction(key, at) as IdentityResult<K>;
    return this.resolveEntity(key, at) as IdentityResult<K>;
  }

  nextBoundary(keys: Iterable<IdentityKey>, hour: number): number {
    const at = Math.floor(hour);
    if (!Number.isFinite(at)) return Infinity;
    let soonest = Infinity;
    for (const key of keys) {
      const result = this.resolve(key, at);
      if ((result.kind === 'known' || result.kind === 'absent') && result.nextBoundary > at && result.nextBoundary < soonest) {
        soonest = result.nextBoundary;
      }
    }
    return soonest;
  }

  private holderForTag(tag: string, at: number): { readonly field: IdentityField<CountryHolder>; readonly interval: HourInterval } {
    if (!this.identities) {
      if (this.identitiesFailure) return { field: { kind: 'failed', failure: this.identitiesFailure }, interval: this.branchBounds };
      return { field: { kind: 'pending' }, interval: this.branchBounds };
    }
    const unk = enclosingGap(this.identitiesUnknown, at, this.branchBounds);
    if (unk.inside) {
      const missing = this.identities.unknown.find(span => span.firstHour <= at && at <= span.lastHour);
      return { field: { kind: 'missing', reason: missing?.reason || 'Country identities were not recorded at this hour' }, interval: unk.interval };
    }
    let bounds = unk.interval;
    if (this.identitiesKnown) {
      const known = enclosingGap(this.identitiesKnown, at, bounds);
      if (!known.inside) {
        return { field: { kind: 'missing', reason: 'Country identities were not recorded at this hour' }, interval: known.interval };
      }
      bounds = known.interval;
    }
    const runs = this.tagIdentities.get(tag) ?? [];
    const idx = findRunBeforeOrAt(runs, at, run => run.firstHour);
    const hit = idx >= 0 ? runs[idx] : undefined;
    if (hit && hit.firstHour <= at && at <= hit.lastHour) {
      const valid = intersectIntervals(bounds, [hit.firstHour, hit.lastHour]);
      return { field: { kind: 'known', value: { slot: hit.slot, countryId: hit.countryId, tag } }, interval: valid };
    }
    const unresolved = this.tagUnresolved.get(tag) ?? [];
    const unres = enclosingGap(unresolved, at, bounds);
    if (unres.inside) {
      return { field: { kind: 'missing', reason: 'Country identity unresolved at this hour' }, interval: unres.interval };
    }
    const prev = idx >= 0 ? runs[idx] : undefined;
    const next = idx + 1 < runs.length ? runs[idx + 1] : undefined;
    const gapFirst = prev ? Math.max(unres.interval[0], prev.lastHour + 1) : unres.interval[0];
    const gapLast = next ? Math.min(unres.interval[1], next.firstHour - 1) : unres.interval[1];
    return { field: { kind: 'missing', reason: NOT_A_COUNTRY }, interval: [gapFirst, gapLast] };
  }

  private resolveCountry(key: CountryIdentityKey, at: number): CountryIdentityResult {
    if (key.by === 'slot' || key.by === 'holder') {
      if (!this.identities) {
        return this.identitiesFailure ? { kind: 'failed', failure: this.identitiesFailure } : { kind: 'pending' };
      }
      const unk = enclosingGap(this.identitiesUnknown, at, this.branchBounds);
      if (unk.inside) {
        const missing = this.identities.unknown.find(span => span.firstHour <= at && at <= span.lastHour);
        const reason = missing?.reason || 'Country identities were not recorded at this hour';
        const field: IdentityField<never> = { kind: 'missing', reason };
        return {
          kind: 'known',
          valid: unk.interval,
          nextBoundary: nextBoundaryOf(unk.interval[1]),
          holder: field,
          tag: field,
          name: field,
          color: field,
          flagChoice: field,
          warnings: this.identities.warnings,
        };
      }
      let bounds = unk.interval;
      if (this.identitiesKnown) {
        const known = enclosingGap(this.identitiesKnown, at, bounds);
        if (!known.inside) {
          const field: IdentityField<never> = { kind: 'missing', reason: 'Country identities were not recorded at this hour' };
          return {
            kind: 'known',
            valid: known.interval,
            nextBoundary: nextBoundaryOf(known.interval[1]),
            holder: field,
            tag: field,
            name: field,
            color: field,
            flagChoice: field,
            warnings: this.identities.warnings,
          };
        }
        bounds = known.interval;
      }
      const allRuns = this.slotRuns.get(key.slot) ?? [];
      const runs = key.by === 'holder' ? allRuns.filter(run => run.countryId === key.countryId) : allRuns;
      const idx = findRunBeforeOrAt(runs, at, run => run.firstHour);
      const run = idx >= 0 ? runs[idx] : undefined;
      if (!run || run.firstHour > at || at > run.lastHour) {
        const prev = idx >= 0 ? runs[idx] : undefined;
        const next = idx + 1 < runs.length ? runs[idx + 1] : undefined;
        const first = prev ? Math.max(bounds[0], prev.lastHour + 1) : bounds[0];
        const last = next ? Math.min(bounds[1], next.firstHour - 1) : bounds[1];
        return {
          kind: 'absent',
          valid: [first, last],
          nextBoundary: nextBoundaryOf(last),
          reason: NOT_A_COUNTRY,
        };
      }
      const holderInterval = intersectIntervals(bounds, [run.firstHour, run.lastHour]);
      const holder: CountryHolder = { slot: key.slot, countryId: run.countryId ?? 0, tag: run.tag };
      const tagResult = this.resolveCountryByTag(run.tag, at, { field: { kind: 'known', value: holder }, interval: holderInterval });
      if (tagResult.kind === 'known') {
        const valid = intersectIntervals(tagResult.valid, holderInterval);
        return {
          ...tagResult,
          valid,
          nextBoundary: nextBoundaryOf(valid[1]),
          holder: { kind: 'known', value: holder },
        };
      }
      if (tagResult.kind === 'absent') {
        const valid = intersectIntervals(tagResult.valid, holderInterval);
        return { ...tagResult, valid, nextBoundary: nextBoundaryOf(valid[1]) };
      }
      return tagResult;
    }
    return this.resolveCountryByTag(key.tag, at);
  }

  private resolveCountryByTag(
    tag: string,
    at: number,
    explicitHolder?: { readonly field: IdentityField<CountryHolder>; readonly interval: HourInterval },
  ): CountryIdentityResult {
    if (this.facts) {
      const spanIdx = findSpanIndex(this.factsKnown, at);
      const span = this.factsKnown[spanIdx];
      if (span && span[0] <= at && at <= span[1]) {
        const runs = this.factsByTag.get(tag);
        const runIdx = runs ? findRunBeforeOrAt(runs, at, item => item[0]) : -1;
        const run = runIdx >= 0 && runs ? runs[runIdx] : undefined;
        if (!run) {
          const first = span[0];
          const last = runs?.length ? Math.min(span[1], runs[0]![0] - 1) : span[1];
          return {
            kind: 'absent',
            valid: [first, last],
            nextBoundary: nextBoundaryOf(last),
            reason: NOT_A_COUNTRY,
          };
        }
        const first = Math.max(span[0], run[0]);
        const nextRunStart = runs && runIdx + 1 < runs.length ? runs[runIdx + 1]![0] : undefined;
        const last = nextRunStart !== undefined ? Math.min(span[1], nextRunStart - 1) : span[1];
        const reason = run[3] >= 0 ? this.facts.reasons[run[3]] ?? null : null;
        if (run[1] < 0 && run[2] < 0 && run[4] < 0 && reason === NOT_A_COUNTRY) {
          return {
            kind: 'absent',
            valid: [first, last],
            nextBoundary: nextBoundaryOf(last),
            reason: NOT_A_COUNTRY,
          };
        }
        const holderInfo = explicitHolder ?? this.holderForTag(tag, at);
        const valid = this.identities || explicitHolder ? intersectIntervals([first, last], holderInfo.interval) : [first, last] as const;
        const name: IdentityField<string> = run[1] >= 0
          ? { kind: 'known', value: this.facts.names[run[1]]! }
          : { kind: 'missing', reason: reason ?? NO_NAME };
        const color: IdentityField<readonly [number, number, number]> = run[2] >= 0
          ? { kind: 'known', value: unpackRgb(run[2]) }
          : { kind: 'missing', reason: NO_COLOUR };
        const flagChoice: IdentityField<CountryFlagChoice> = run[4] >= 0
          ? { kind: 'known', value: this.facts.flags[run[4]]! }
          : { kind: 'missing', reason: reason ?? NO_FLAG };
        const warnings = reason === null ? this.facts.warnings : [...this.facts.warnings, `${tag}: ${reason}`];
        return {
          kind: 'known',
          valid,
          nextBoundary: nextBoundaryOf(valid[1]),
          holder: holderInfo.field,
          tag: { kind: 'known', value: tag },
          name,
          color,
          flagChoice,
          warnings,
        };
      }
      if (this.factsFinal) {
        const prev = spanIdx > 0 ? this.factsKnown[spanIdx - 1] : undefined;
        const first = prev ? Math.max(this.branchBounds[0], prev[1] + 1) : this.branchBounds[0];
        const last = span ? Math.min(this.branchBounds[1], span[0] - 1) : this.branchBounds[1];
        const missing: IdentityField<never> = { kind: 'missing', reason: FACTS_NOT_RECORDED };
        const holderInfo = explicitHolder ?? this.holderForTag(tag, at);
        const valid = this.identities || explicitHolder ? intersectIntervals([first, last], holderInfo.interval) : [first, last] as const;
        return {
          kind: 'known',
          valid,
          nextBoundary: nextBoundaryOf(valid[1]),
          holder: holderInfo.field,
          tag: { kind: 'known', value: tag },
          name: missing,
          color: missing,
          flagChoice: missing,
          warnings: this.facts.warnings,
        };
      }
    }

    const style = findCoveringStyle(this.styleRuns.get(tag), at);
    if (style) {
      const baseValid: HourInterval = [style.firstHour, style.lastHour];
      if (style.absent) {
        return {
          kind: 'absent',
          valid: baseValid,
          nextBoundary: nextBoundaryOf(baseValid[1]),
          reason: style.reason ?? NOT_A_COUNTRY,
        };
      }
      const holderInfo = explicitHolder ?? this.holderForTag(tag, at);
      const valid = this.identities || explicitHolder ? intersectIntervals(baseValid, holderInfo.interval) : baseValid;
      const name: IdentityField<string> = style.name !== null
        ? { kind: 'known', value: style.name }
        : { kind: 'missing', reason: style.reason ?? NO_NAME };
      const color: IdentityField<readonly [number, number, number]> = style.color !== null
        ? { kind: 'known', value: style.color }
        : { kind: 'missing', reason: style.reason ?? NO_COLOUR };
      const flagChoice: IdentityField<CountryFlagChoice> = style.flagChoice === undefined
        ? { kind: 'pending' }
        : style.flagChoice !== null
          ? { kind: 'known', value: style.flagChoice }
          : { kind: 'missing', reason: style.reason ?? NO_FLAG };
      return {
        kind: 'known',
        valid,
        nextBoundary: nextBoundaryOf(valid[1]),
        holder: holderInfo.field,
        tag: { kind: 'known', value: tag },
        name,
        color,
        flagChoice,
        warnings: style.warnings,
      };
    }

    const styleFailure = this.styleFailures.get(`${at}\u0000${tag}`);
    if (styleFailure) return { kind: 'failed', failure: styleFailure };
    if (this.factsFailure) return { kind: 'failed', failure: this.factsFailure };
    return { kind: 'pending' };
  }

  private resolveFaction(key: FactionIdentityKey, at: number): FactionIdentityResult {
    if (!this.groups) {
      return this.groupsFailure ? { kind: 'failed', failure: this.groupsFailure } : { kind: 'pending' };
    }
    const obs = enclosingGap(this.factionsUnobserved, at, this.branchBounds);
    const faction = this.factionsById.get(key.id);
    if (obs.inside) {
      const missing: IdentityField<never> = { kind: 'missing', reason: FACTIONS_NOT_RECORDED };
      return {
        kind: 'known',
        valid: obs.interval,
        nextBoundary: nextBoundaryOf(obs.interval[1]),
        id: key.id,
        recordedName: faction?.recordedName ?? '',
        name: missing,
        leader: missing,
        color: missing,
        incomplete: true,
        warnings: this.groups.warnings,
      };
    }
    if (!faction) {
      return {
        kind: 'absent',
        valid: obs.interval,
        nextBoundary: nextBoundaryOf(obs.interval[1]),
        reason: FACTION_ABSENT,
      };
    }
    if (at < faction.firstHour) {
      const valid: HourInterval = [obs.interval[0], Math.min(obs.interval[1], faction.firstHour - 1)];
      return { kind: 'absent', valid, nextBoundary: nextBoundaryOf(valid[1]), reason: FACTION_ABSENT };
    }
    if (at > faction.lastHour) {
      const valid: HourInterval = [Math.max(obs.interval[0], faction.lastHour + 1), obs.interval[1]];
      return { kind: 'absent', valid, nextBoundary: nextBoundaryOf(valid[1]), reason: FACTION_ABSENT };
    }
    let valid = intersectIntervals(obs.interval, [faction.firstHour, faction.lastHour]);

    const nameIdx = findRunBeforeOrAt(faction.names, at, run => run.firstHour);
    const nameRun = nameIdx >= 0 ? faction.names[nameIdx] : undefined;
    let name: IdentityField<string>;
    if (nameRun && nameRun.firstHour <= at && at <= nameRun.lastHour) {
      valid = intersectIntervals(valid, [nameRun.firstHour, nameRun.lastHour]);
      name = nameRun.name !== null
        ? { kind: 'known', value: nameRun.name }
        : { kind: 'missing', reason: nameRun.reason ?? 'Faction name not recorded at this hour' };
    } else {
      const prev = nameIdx >= 0 ? faction.names[nameIdx] : undefined;
      const next = nameIdx + 1 < faction.names.length ? faction.names[nameIdx + 1] : undefined;
      valid = intersectIntervals(valid, [prev ? prev.lastHour + 1 : valid[0], next ? next.firstHour - 1 : valid[1]]);
      name = { kind: 'missing', reason: 'Faction name not recorded at this hour' };
    }

    const leaderIdx = findRunBeforeOrAt(faction.leaders, at, run => run.firstHour);
    const leaderRun = leaderIdx >= 0 ? faction.leaders[leaderIdx] : undefined;
    let leader: IdentityField<string>;
    let color: IdentityField<readonly [number, number, number]>;
    if (leaderRun && leaderRun.firstHour <= at && at <= leaderRun.lastHour) {
      valid = intersectIntervals(valid, [leaderRun.firstHour, leaderRun.lastHour]);
      leader = { kind: 'known', value: leaderRun.tag };
      const leaderCountry = this.resolveCountry(countryTagKey(leaderRun.tag), at);
      if (leaderCountry.kind === 'known') {
        valid = intersectIntervals(valid, leaderCountry.valid);
        color = leaderCountry.color;
      } else if (leaderCountry.kind === 'absent') {
        valid = intersectIntervals(valid, leaderCountry.valid);
        color = { kind: 'missing', reason: leaderCountry.reason };
      } else if (leaderCountry.kind === 'failed') {
        color = { kind: 'failed', failure: leaderCountry.failure };
      } else {
        color = { kind: 'pending' };
      }
    } else {
      const prev = leaderIdx >= 0 ? faction.leaders[leaderIdx] : undefined;
      const next = leaderIdx + 1 < faction.leaders.length ? faction.leaders[leaderIdx + 1] : undefined;
      valid = intersectIntervals(valid, [prev ? prev.lastHour + 1 : valid[0], next ? next.firstHour - 1 : valid[1]]);
      leader = { kind: 'missing', reason: 'Faction leader not recorded at this hour' };
      color = { kind: 'missing', reason: 'Faction leader not recorded at this hour' };
    }

    const incSpans = this.factionIncomplete.get(key.id) ?? [];
    const inc = enclosingGap(incSpans, at, valid);
    valid = intersectIntervals(valid, inc.interval);

    return {
      kind: 'known',
      valid,
      nextBoundary: nextBoundaryOf(valid[1]),
      id: faction.id,
      recordedName: faction.recordedName,
      name,
      leader,
      color,
      incomplete: inc.inside !== null,
      warnings: this.groups.warnings,
    };
  }

  private resolveEntity(key: EntityIdentityKey, at: number): EntityIdentityResult {
    const id = identityKeyString(key);
    const entry = findCoveringEntity(this.entityRuns.get(id), at);
    if (entry) {
      const valid: HourInterval = [entry.firstHour, entry.lastHour];
      if (entry.absent) {
        return {
          kind: 'absent',
          valid,
          nextBoundary: nextBoundaryOf(valid[1]),
          reason: entry.reason ?? `${key.kind} does not exist at this hour`,
        };
      }
      const name: IdentityField<string> = entry.name !== null
        ? { kind: 'known', value: entry.name }
        : { kind: 'missing', reason: entry.nameReason ?? `${key.kind} name unavailable` };
      const subtitle: IdentityField<string> | undefined = entry.subtitle === undefined
        ? undefined
        : entry.subtitle !== null
          ? { kind: 'known', value: entry.subtitle }
          : { kind: 'missing', reason: entry.subtitleReason ?? `${key.kind} subtitle unavailable` };
      const artKey: IdentityField<string> | undefined = entry.artKey === undefined
        ? undefined
        : entry.artKey !== null
          ? { kind: 'known', value: entry.artKey }
          : { kind: 'missing', reason: entry.artReason ?? `${key.kind} art unavailable` };
      return {
        kind: 'known',
        entityKind: key.kind,
        valid,
        nextBoundary: nextBoundaryOf(valid[1]),
        name,
        ...(subtitle ? { subtitle } : {}),
        ...(artKey ? { artKey } : {}),
        warnings: entry.warnings ?? [],
      };
    }
    const failure = this.entityFailures.get(id);
    if (failure) return { kind: 'failed', failure };
    return { kind: 'pending' };
  }
}

export * from './types.ts';
