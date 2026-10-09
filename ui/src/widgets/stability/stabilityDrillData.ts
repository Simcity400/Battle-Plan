import { pickLines } from '../../charts/linePick.ts';
import type { ChartPick } from '../../charts/pick';
import { pinnedBreakdown } from '../../charts/drill/drillState.ts';
import type { Breakdown, DrillAction } from '../../charts/drill/drillState';
import { clipRuns, isMember } from '../../dashboard/groups.ts';
import { percentRuns, rawAt } from './stabilityData.ts';
import type { MembershipRun } from '../../generated/dashboard';
import { clipToSpans, stretchAt, stretchStarts } from '../../charts/series.ts';
import type { Run, Span } from '../../charts/series';
import type { WidgetSpec, WidgetTime } from '../../dashboard/model';
import type { CountryStabilityReply, StabilityPartsReply } from '../../generated/dashboard';
import type { Measure } from './stabilityData';

export interface StabilityLine { tag: string; measure: Measure; runs: readonly Run[] }
export interface StabilityPick {
  hour: number; candidates: readonly string[]; lines: readonly { tag: string; measure: Measure }[];
  tag: string | null; measure: Measure;
}

/** Actor and measure come only from the curves witnessed in the drawn frame. */
export function pickStabilityLine(lines: readonly StabilityLine[], hour: number, context?: ChartPick,
  preferred: Measure = 'stability'): StabilityPick | null {
  if (!context?.frame) return null;
  const keyed = lines.map(line => ({ ...line, key: JSON.stringify([line.tag, line.measure]) }));
  const keys = new Set(pickLines(keyed, hour, context));
  const witnessed = keyed.filter(line => keys.has(line.key)).map(({ tag, measure }) => ({ tag, measure }));
  const candidates = [...new Set(witnessed.map(line => line.tag))];
  if (!candidates.length) return null;
  return { hour: Math.floor(hour), candidates, lines: witnessed, tag: candidates.length === 1 ? candidates[0] : null,
    measure: witnessed.find(line => line.measure === preferred)?.measure ?? witnessed[0].measure };
}

export function chooseStabilityActor(pick: StabilityPick, tag: string | null): StabilityPick {
  if (!tag || !pick.candidates.includes(tag)) return pick;
  const lines = pick.lines.filter(line => line.tag === tag);
  return { ...pick, tag, measure: lines.find(line => line.measure === pick.measure)?.measure ?? lines[0].measure };
}

/** A campaign, saved pin or range has its own moment. Only a Follow widget's
 * initial open can inherit playback; an overlap stays held until its choice. */
export function stabilityPickAction(pick: StabilityPick, open: boolean, time: WidgetTime['mode'], playing: boolean,
  playhead: number | null): DrillAction {
  return open ? { type: 'pick', hour: pick.hour } : { type: 'open', breakdown: 'parts', hour: pick.hour,
    playing: pick.tag !== null && time === 'follow' && playing,
    playhead: pick.tag !== null && time === 'follow' ? playhead : null };
}

/** Pin uses the chosen actor and raw branch hour, independent of axis alignment. */
export function pinStabilityDrill(widget: WidgetSpec, breakdown: Breakdown, trackId: string, firstHour: number,
  tag: string, hour: number, following: boolean): WidgetSpec {
  return pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [tag] }, time: following ? { mode: 'follow' }
    : { mode: 'pin', windows: { [trackId]: { firstOffset: Math.floor(hour) - firstHour, lastOffset: Math.floor(hour) - firstHour } } } }, breakdown, trackId);
}

/** Content labels and artwork never own the recorded numeric geometry. */
export const stabilityNumbers = (reply: CountryStabilityReply): CountryStabilityReply => ({ ...reply, warnings: [],
  series: reply.series.map(row => ({ ...row, maxCommandPower: [], breakdown: null })) });
export const stabilityPartsNumbers = (reply: StabilityPartsReply): StabilityPartsReply => ({ ...reply, warnings: [],
  stability: { ...reply.stability, parts: reply.stability.parts.map(part => ({ ...part, line: null })) },
  warSupport: { ...reply.warSupport, parts: reply.warSupport.parts.map(part => ({ ...part, line: null })) },
});

/** Branch geometry is prepared once per numeric reply and membership/range. */
export function stabilityPlotSeries(reply: CountryStabilityReply | undefined,
  membership: ReadonlyMap<string, readonly MembershipRun[]> | null, extent: readonly [number, number]) {
  return (reply?.series ?? []).map(item => {
    const spans = membership?.get(item.tag);
    const shown: MembershipRun[] = [{ tag: item.tag, firstHour: extent[0], lastHour: extent[1] }];
    const line = (measure: Measure) => {
      const runs = clipRuns(percentRuns(item[measure], reply!.decimals), shown);
      return spans ? clipRuns(runs, spans) : runs;
    };
    return { tag: item.tag, carried: item.carried, lines: { stability: line('stability'), warSupport: line('warSupport') },
      valueAt: (measure: Measure, hour: number) => hour >= extent[0] && hour <= extent[1] && isMember(spans, hour) ? rawAt(item[measure], hour) : null };
  });
}

/** Numeric native identity/data boundaries guard month samples and held looks,
 * independent of display membership. Either known measure supplies a bound. */
export function stabilityLookSegments(reply: CountryStabilityReply | undefined) {
  const index = new Map((reply?.series ?? []).map(row => [row.tag, [
    { runs: row.stability, starts: stretchStarts(row.stability) },
    { runs: row.warSupport, starts: stretchStarts(row.warSupport) },
  ]]));
  return (tag: string, hour: number): number | null => {
    const starts = index.get(tag)?.flatMap(({ runs, starts }) => {
      const start = stretchAt(runs, starts, hour);
      return start === null ? [] : [start];
    }) ?? [];
    return starts.length ? Math.max(...starts) : null;
  };
}

/** Alias histories retain their supplying reads and never include a reused tag's parts. */
export function stabilityCountryParts(replies: readonly StabilityPartsReply[], spans: ReadonlyMap<string, readonly Span[]>, tag: string): StabilityPartsReply | undefined {
  if (!replies.length) return undefined;
  const measure = (key: Measure) => {
    const parts = new Map<string, StabilityPartsReply[Measure]['parts'][number]>();
    const observed = replies.flatMap(reply => {
      const known = spans.get(reply.tag) ?? [];
      for (const part of reply[key].parts) {
        const runs = clipToSpans(part.runs, known);
        const previous = parts.get(part.id);
        if (previous) previous.runs.push(...runs); else parts.set(part.id, { ...part, runs });
      }
      return clipToSpans(reply[key].observed, known);
    });
    return { parts: [...parts.values()].map(part => ({ ...part, runs: part.runs.sort((a, b) => a.firstHour - b.firstHour) })),
      observed: observed.sort((a, b) => a.firstHour - b.firstHour) };
  };
  return { ...replies[0], tag, stability: measure('stability'), warSupport: measure('warSupport'), warnings: replies.flatMap(reply => reply.warnings) };
}
