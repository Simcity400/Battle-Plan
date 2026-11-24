import type { BoundTrack, CountrySelection } from '../dashboard/model';
import type { Gate } from '../dashboard/regionGate';
import { coverageSpan, recordedHour, type BoundWindow, type GatedHour } from '../dashboard/regionTime.ts';
import type { CountryGroupReply, HourSpan } from '../generated/dashboard';
import { nextFactionsChange, type FamilyFactionRun, type HourRanges } from './holders.ts';

export { nextFactionsChange };
export type { FamilyFactionRun };

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
