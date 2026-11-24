import type { BoundTrack, CountrySelection, WidgetSpec } from '../dashboard/model';
import { selectedCountryKeys } from '../dashboard/countrySelection.ts';
import type { Gate } from '../dashboard/regionGate';
import { coverageSpan, recordedHour, type BoundWindow, type GatedHour } from '../dashboard/regionTime.ts';
import { authoredHex } from '../charts/palette.ts';
import type { CountryFlagChoice, CountryGroupReply, HourSpan } from '../generated/dashboard';
import { type FamilyFactionRun, type HourRanges } from './holders.ts';
import { countryTagKey, type DrawnFlagEntry, type IdentityIndex } from './identity/index.ts';
import type { CountryLook, CountryLooksMap } from './looks.ts';

export interface FrozenGate {
  readonly recorded: Gate['recorded'];
  readonly span: Gate['span'];
  readonly recordedRun?: (hour: number) => readonly [number, number];
}

export function freezeGate(
  gate: Pick<Gate, 'recorded' | 'span'>,
  recordedRun?: (hour: number) => readonly [number, number],
): FrozenGate {
  return recordedRun ? { recorded: gate.recorded, span: gate.span, recordedRun } : { recorded: gate.recorded, span: gate.span };
}

export function trackGate(track: Pick<BoundTrack, 'branch' | 'source'>): FrozenGate {
  const { firstHour, lastHour } = track.branch;
  return {
    recorded: hour => recordedHour(track, hour) as GatedHour | null,
    span: (first, last) => {
      const from = Math.max(Math.floor(first), firstHour), to = Math.min(Math.floor(last), lastHour);
      return from <= to ? { firstHour: from, lastHour: to } as BoundWindow : null;
    },
    recordedRun: hour => coverageSpan(track, hour),
  };
}

export function planOwner(
  kind: string,
  track: BoundTrack | undefined,
  time: WidgetSpec['time'],
  countries: CountrySelection,
  single: string | null | undefined,
): string {
  return JSON.stringify([
    kind,
    track?.source.id ?? '',
    track?.source.revision ?? '',
    track?.branch.id ?? '',
    track?.axisOffset ?? 0,
    time,
    single !== undefined ? single : countries.kind === 'selected' ? { kind: 'selected', keys: selectedCountryKeys(countries, track) } : countries,
  ]);
}

export function effectiveCountryUnobserved(
  selection: CountrySelection,
  raw: readonly HourSpan[] | undefined,
): HourRanges {
  if (!raw?.length) return [];
  if (selection.kind !== 'group' || selection.membership !== 'hour') return [];
  return raw.map(span => [span.firstHour, span.lastHour] as const);
}

export function selectionUnobservedReason(
  selection: CountrySelection,
  raw: readonly HourSpan[] | undefined,
): string | null {
  if (!raw?.length) return null;
  if (selection.kind === 'group' && selection.membership === 'ever') {
    return 'Group membership is incomplete for this campaign.';
  }
  return null;
}

export interface FamilyFactionData {
  readonly runs: readonly FamilyFactionRun[] | null;
  readonly unobserved: HourRanges;
  readonly incomplete: HourRanges;
}

const NO_RANGES: HourRanges = [];

function mergeHourSpans(spans: readonly HourSpan[]): HourRanges {
  if (!spans.length) return NO_RANGES;
  const sorted = [...spans].sort((a, b) => a.firstHour - b.firstHour || a.lastHour - b.lastHour);
  const out: [number, number][] = [];
  for (const span of sorted) {
    if (span.firstHour > span.lastHour) continue;
    const prev = out[out.length - 1];
    if (prev && prev[1] + 1 >= span.firstHour) {
      if (span.lastHour > prev[1]) prev[1] = span.lastHour;
    } else {
      out.push([span.firstHour, span.lastHour]);
    }
  }
  return out;
}

export function extractFactionData(groups: CountryGroupReply | undefined, enabled: boolean): FamilyFactionData {
  if (!enabled || !groups) {
    return { runs: null, unobserved: NO_RANGES, incomplete: NO_RANGES };
  }
  const runs: FamilyFactionRun[] = groups.factions.flatMap(faction => faction.members.map(run => ({
    id: faction.id,
    tag: run.tag,
    firstHour: run.firstHour,
    lastHour: run.lastHour,
    names: faction.names,
  })));
  const unobserved = mergeHourSpans(groups.unobserved.factions);
  const allIncomplete: HourSpan[] = [];
  for (const faction of groups.factions) {
    if (faction.incomplete?.length) allIncomplete.push(...faction.incomplete);
  }
  const incomplete = mergeHourSpans(allIncomplete);
  return { runs, unobserved, incomplete };
}

export function buildCountryLooksFromIdentity(
  index: IdentityIndex | null,
  tags: readonly string[],
  firstHour: number,
  lastHour: number,
  flagFor: (choice: CountryFlagChoice) => DrawnFlagEntry | undefined,
  includeColours: boolean,
): CountryLooksMap {
  if (!index || firstHour > lastHour) return {};
  const out: Record<string, CountryLook[]> = {};
  for (const tag of tags) {
    const key = countryTagKey(tag);
    const runs: CountryLook[] = [];
    let cursor = Math.floor(firstHour);
    const limit = Math.floor(lastHour);
    while (cursor <= limit) {
      const res = index.resolve(key, cursor);
      if (res.kind === 'known') {
        const name = res.name.kind === 'known' ? res.name.value : null;
        const status = res.name.kind === 'missing'
          ? res.name.reason
          : res.name.kind === 'failed'
            ? res.name.failure.message
            : res.name.kind === 'pending'
              ? 'Loading…'
              : undefined;
        const color = includeColours && res.color.kind === 'known' ? authoredHex(res.color.value) : undefined;
        let flag: string | undefined;
        let flagPending = false;
        if (res.flagChoice.kind === 'known') {
          const drawn = flagFor(res.flagChoice.value);
          if (!drawn) flagPending = true;
          else if (drawn.url) flag = drawn.url;
        } else if (res.flagChoice.kind === 'pending') {
          flagPending = true;
        }
        runs.push({
          from: cursor,
          name,
          ...(flag ? { flag } : {}),
          ...(flagPending ? { flagPending: true } : {}),
          ...(color ? { color } : {}),
          ...(status ? { status } : {}),
        });
        cursor = Number.isFinite(res.nextBoundary) && res.nextBoundary > cursor ? res.nextBoundary : limit + 1;
      } else if (res.kind === 'absent') {
        runs.push({ from: cursor, name: null, status: res.reason });
        cursor = Number.isFinite(res.nextBoundary) && res.nextBoundary > cursor ? res.nextBoundary : limit + 1;
      } else if (res.kind === 'failed') {
        runs.push({ from: cursor, name: null, status: res.failure.message });
        break;
      } else {
        runs.push({ from: cursor, name: null, flagPending: true, status: 'Loading…' });
        break;
      }
    }
    if (runs.length) out[tag] = runs;
  }
  return out;
}
