import type { BoundTrack, CountrySelection, WidgetSpec } from '../dashboard/model';
import { selectedCountryKeys } from '../dashboard/countrySelection.ts';
import { everKnownBefore } from '../dashboard/groups.ts';
import type { Gate } from '../dashboard/regionGate';
import { coverageSpan, recordedHour, type BoundWindow, type GatedHour } from '../dashboard/regionTime.ts';
import type { CountryGroupReply, HourSpan } from '../generated/dashboard';
import type { FamilyFactionRun, HourRanges } from './holders.ts';

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
  if (!raw?.length || selection.kind !== 'group') return [];
  if (selection.membership === 'ever') return [[everKnownBefore(raw), Number.MAX_SAFE_INTEGER]];
  return raw.map(span => [span.firstHour, span.lastHour] as const);
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
