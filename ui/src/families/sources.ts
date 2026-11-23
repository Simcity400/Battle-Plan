import { coloursIn, lookStarts, type CountryTimeline } from '../dashboard/countryTimeline.ts';
import type { CountrySelection } from '../dashboard/model';
import type { Gate } from '../dashboard/regionGate';
import type { CountryGroupReply, HourSpan } from '../generated/dashboard';
import { nextFactionsChange, type FamilyFactionRun, type HourRanges } from './holders.ts';
import type { CountryLook, CountryLooksMap } from './looks.ts';

export { nextFactionsChange };
export type { FamilyFactionRun };

export interface FrozenGate {
  readonly recorded: Gate['recorded'];
  readonly span: Gate['span'];
}

export function freezeGate(gate: Pick<Gate, 'recorded' | 'span'>): FrozenGate {
  return { recorded: gate.recorded, span: gate.span };
}

export function resolveRetainedHeadSpan(
  moment: string | undefined,
  fallback: readonly [number, number] | null,
): readonly [number, number] | null {
  if (moment) {
    try {
      const parsed = JSON.parse(moment) as unknown;
      if (Array.isArray(parsed) && typeof parsed[0] === 'number' && typeof parsed[1] === 'number') {
        return [parsed[0], parsed[1]] as const;
      }
    } catch {
      return fallback;
    }
  }
  return fallback;
}

export interface DisplayedStaticSnapshot<TContext = unknown, TExtra = unknown> {
  readonly data: unknown;
  readonly moment: string | undefined;
  readonly requestKey: string | null;
  readonly span: readonly [number, number] | null;
  readonly context: TContext;
  readonly gate: FrozenGate;
  readonly extra: TExtra;
}

export interface DisplayedStaticInput<TContext, TExtra> {
  readonly data: unknown;
  readonly moment: string | undefined;
  readonly requestKey: string | null;
  readonly satisfiesCurrent: boolean;
  readonly requestedSpan: readonly [number, number] | null;
  readonly context: TContext;
  readonly gate: FrozenGate;
  readonly extra: TExtra;
}

export function resolveDisplayedStaticSnapshot<TContext, TExtra = undefined>(
  previous: DisplayedStaticSnapshot<TContext, TExtra> | null,
  input: DisplayedStaticInput<TContext, TExtra>,
): DisplayedStaticSnapshot<TContext, TExtra> | null {
  if (input.data === undefined) return null;
  if (input.satisfiesCurrent) {
    return {
      data: input.data,
      moment: input.moment,
      requestKey: input.requestKey,
      span: resolveRetainedHeadSpan(input.moment, input.requestedSpan),
      context: input.context,
      gate: input.gate,
      extra: input.extra,
    };
  }
  if (previous && previous.data === input.data && previous.moment === input.moment) {
    return previous;
  }
  return {
    data: input.data,
    moment: input.moment,
    requestKey: input.requestKey,
    span: resolveRetainedHeadSpan(input.moment, input.requestedSpan),
    context: input.context,
    gate: input.gate,
    extra: input.extra,
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

export function colourStringFromPacked(packed: number | undefined): string | undefined {
  if (packed === undefined || packed < 0) return undefined;
  return `rgb(${(packed >> 16) & 255}, ${(packed >> 8) & 255}, ${packed & 255})`;
}

export function buildDatedCountryLooks(
  timeline: CountryTimeline | undefined,
  tags: readonly string[],
  firstHour: number,
  lastHour: number,
  exactAt: (hour: number) => (tag: string) => { readonly known?: boolean; readonly name: string; readonly flag?: string } | undefined,
  includeColours: boolean,
): CountryLooksMap {
  const out: Record<string, CountryLook[]> = {};
  for (const tag of tags) {
    const starts = lookStarts(timeline, tag, firstHour, lastHour);
    const runs: CountryLook[] = [];
    for (const [at, held] of starts) {
      const look = held ? exactAt(at)(tag) : undefined;
      const packed = held && includeColours && timeline ? coloursIn(timeline, at, [tag])?.get(tag) : undefined;
      const color = colourStringFromPacked(packed);
      runs.push({
        from: at,
        name: look?.known ? look.name : null,
        ...(look?.flag ? { flag: look.flag } : {}),
        ...(color ? { color } : {}),
        ...(look && !look.known ? { status: look.name } : {}),
      });
    }
    out[tag] = runs;
  }
  return out;
}
