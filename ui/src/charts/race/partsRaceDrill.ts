import type { GatedTime } from '../../dashboard/regionTime';
import { CLOSED, pinnedBreakdown, reduceDrill, type PinnedSpec } from '../drill/drillState.ts';
import type { DrillAction, DrillState } from '../drill/drillState';
import { shownRuns } from './lines.ts';
import type { Run, Span } from '../series';
import { partsRaceAt, partsRaceSegmentAt, validatePartsRaceReply } from '../../dashboard/partsRace.ts';
import type { PartsRaceModel, PartsRaceReply, PartsRaceSeries } from '../../dashboard/partsRace';
import { validCountryIdentityHistories } from '../../dashboard/countryIdentity.ts';
import type { CountryIdentityRun } from '../../generated/dashboard';
import type { CountryPartsData, CountryPartsSelection } from './countryPartsDrillData';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';

export const UNATTRIBUTED_PART = { id: '', label: 'Other' };

export interface NativePartsRaceReply extends PartsRaceReply {
  series: (PartsRaceSeries & { identities: CountryIdentityRun[] })[];
}

export function validateNativePartsRaceReply(value: unknown): NativePartsRaceReply {
  const reply = validatePartsRaceReply(value);
  if (!validCountryIdentityHistories(reply.series)) throw new Error('The parts race identity history is malformed.');
  return reply as NativePartsRaceReply;
}

export function partsRaceDrillData(model: PartsRaceModel, histories: NativePartsRaceReply['series'],
  tooltip: CountryPartsData['tooltip']): CountryPartsData {
  return { histories, warnings: model.reply.warnings,
    parts: () => model.reply.parts.map(part => ({ id: part.key, label: part.name ?? 'Part name unavailable' })),
    at: (tag, hour) => partsRaceAt(model, tag, hour),
    bands: (tag, range) => partsRaceDrillBands(model, tag, null, range),
    segmentAt: (tag, hour) => partsRaceSegmentAt(model, tag, hour),
    readHour: (tag, hour) => partsRaceAt(model, tag, hour) ? hour : null,
    keyHours: [...new Set(histories.flatMap(series => series.runs.map(run => run.firstHour)))].sort((a, b) => a - b),
    tooltip };
}

export function partsRaceDrillActors(tag: string | undefined, choices: readonly string[] | undefined,
  fallback: () => string | null): readonly string[] {
  if (choices !== undefined) return [...new Set(choices)];
  const country = tag ?? fallback();
  return country ? [country] : [];
}

export interface PartsRaceDrillState extends DrillState {
  choices: { key: string; tags: readonly string[] } | null;
  selection?: CountryPartsSelection;
  owner?: unknown;
  pickOwner?: unknown;
}
export const CLOSED_PARTS_RACE: PartsRaceDrillState = { ...CLOSED, choices: null };
export type PartsRaceDrillAction = DrillAction
  | { type: 'actors'; key: string; tags: readonly string[]; hour: number }
  | { type: 'choose'; key: string; tag: string; hour: number };
export type NativePartsRaceDrillAction = PartsRaceDrillAction & { selection?: CountryPartsSelection; owner?: unknown; pickOwner?: unknown };

export function reducePartsRaceDrill(state: PartsRaceDrillState, action: NativePartsRaceDrillAction): PartsRaceDrillState {
  if (action.type === 'actors') return { ...CLOSED, spec: false, follow: false, hour: Math.floor(action.hour),
    choices: { key: action.key, tags: action.tags }, ...(action.owner ? { owner: action.owner } : {}),
    ...(action.pickOwner ? { pickOwner: action.pickOwner } : {}) };
  if (action.type === 'choose') {
    if (!state.choices || state.choices.key !== action.key || state.hour !== action.hour || !state.choices.tags.includes(action.tag)) return state;
    return { ...reduceDrill(state, { type: 'open', breakdown: action.tag, hour: action.hour, follow: false }), choices: null,
      ...(action.selection ? { selection: action.selection } : {}), ...(action.owner ? { owner: action.owner } : {}) };
  }
  if (state.choices && (action.type === 'follow' || action.type === 'pick')) return state;
  const selection = action.selection ?? (action.type === 'open' ? undefined : state.selection);
  return { ...reduceDrill(state, action), choices: null,
    ...(action.type !== 'close' && selection ? { selection } : {}),
    ...(action.type !== 'close' && (action.owner ?? state.owner) ? { owner: action.owner ?? state.owner } : {}) };
}

export function partsRaceDrillBands(model: PartsRaceModel, tag: string, membership: readonly Span[] | null, range: Span): Run[][] {
  return Array.from({ length: model.reply.parts.length + 1 }, (_, index) => {
    const runs: Run[] = [];
    for (const run of model.series.get(tag)?.series.runs ?? []) {
      if (run.total == null) continue;
      const value = index < model.reply.parts.length ? run.values[index] ?? 0
        : run.total - run.values.reduce<number>((sum, part) => sum + (part ?? 0), 0);
      const previous = runs.at(-1);
      if (previous && !run.startsSegment && previous.lastHour + 1 === run.firstHour && previous.value === value) previous.lastHour = run.lastHour;
      else runs.push({ firstHour: run.firstHour, lastHour: run.lastHour, value, ...(run.startsSegment ? { breakBefore: true } : {}) });
    }
    return shownRuns(runs, membership, range);
  });
}

export function pinPartsRace(source: WidgetSpec, track: Pick<BoundTrack, 'id'>, tag: string, time: GatedTime, label: string): PinnedSpec {
  return { ...pinnedBreakdown(source, { id: 'breakdown', label }, track.id, time), countries: { kind: 'tags', tags: [tag] } };
}
