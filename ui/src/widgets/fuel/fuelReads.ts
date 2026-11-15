import { useCallback, useEffect, useMemo } from 'react';
import type { ChartSummary } from '../../charts/chartSummary';
import { LiveCell } from '../../charts/live';
import type { TimeChartProps } from '../../charts/TimeChart';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { Failure } from '../../dashboard/queryError';
import type { CountryFuelReply, HourSpan } from '../../generated/dashboard';
import { useMeasureSeries } from '../country-measure/measureSeries';
import { COUNTRY_FUEL, validateCountryFuel } from './fuelData';

export interface FuelAt {
  /** The reply rendered: the structure while followed, the hour's reply while held. */
  data?: CountryFuelReply;
  error?: Failure;
  retry: () => void;
  /** Followed, the newest reply pushed, painted without a render; held, the hour's reply. */
  live: LiveCell<CountryFuelReply | undefined>;
  /** The element the followed panel is shown in. */
  root: (element: Element | null) => void;
  followed: boolean;
  /** The hour the reply shown is of, when known. */
  shownAt: () => number | null;
}

/** A country's fuel at a moment: followed at the head as a pushed panel (a render only when `structure` of it changes,
 * its numbers painted from `live`), or asked once for a held hour. */
export function useFuelAt(runtime: WidgetRuntime, widget: WidgetSpec, track: BoundTrack, tag: string | null, hour: number | null, follows: boolean,
  active: boolean, structure: (reply: CountryFuelReply) => string): FuelAt {
  const live = useMemo(() => new LiveCell<CountryFuelReply | undefined>(undefined), []);
  const spec = useMemo<WidgetSpec>(() => ({ ...widget, time: { mode: 'follow' } } as WidgetSpec), [widget]);
  const paint = useCallback((reply: CountryFuelReply) => live.set(reply), [live]);
  const head = useHeadProduct<CountryFuelReply>(runtime, spec, COUNTRY_FUEL, follows && tag ? { tag } : null,
    { active: active && follows, validate: validateCountryFuel, structure, paint });
  const asked = useProduct<CountryFuelReply>(runtime.client, !follows && tag && hour !== null ? { sourceId: track.source.id, revision: track.source.revision,
    product: COUNTRY_FUEL, params: { branchId: track.branch.id, tag, hour } } : null, active && !follows, validateCountryFuel, track.source);
  useEffect(() => { live.set(follows ? head.data : asked.data); }, [live, follows, head.data, asked.data]);
  return follows
    ? { data: head.data, error: head.error, retry: head.retry, live, root: head.root, followed: true, shownAt: head.shownAt }
    : { data: asked.data, error: asked.error, retry: asked.retry, live, root: NO_ROOT, followed: false, shownAt: () => asked.data ? hour : null };
}

const NO_ROOT = () => {};

/** The most series one breakdown chart reads at once. */
export const MOST_PARTS = 8;

/** Some measures of one declared series for one country, from tiles over the chart's view: `MOST_PARTS` slots, each a
 * measure or none, so the hooks called never change. A tile landing renders: a breakdown is not followed at the head. */
export function usePartSeries(runtime: WidgetRuntime, track: BoundTrack, kind: string, measures: readonly (string | null)[], tag: string | null,
  within: ReadonlyMap<string, readonly HourSpan[]> | undefined, active: boolean) {
  const args = (at: number) => {
    const measure = measures[at] ?? null;
    return [runtime.client, measure && tag ? track.source : undefined, track.branch.id, kind, measure ?? '', tag ? [tag] : [],
      null, null, active && !!measure && !!tag, false, within, track.branch.lastHour, false] as const;
  };
  const s0 = useMeasureSeries(...args(0)), s1 = useMeasureSeries(...args(1)), s2 = useMeasureSeries(...args(2)), s3 = useMeasureSeries(...args(3));
  const s4 = useMeasureSeries(...args(4)), s5 = useMeasureSeries(...args(5)), s6 = useMeasureSeries(...args(6)), s7 = useMeasureSeries(...args(7));
  const slots = [s0, s1, s2, s3, s4, s5, s6, s7];
  const onSummaryView = useCallback<NonNullable<TimeChartProps['onSummaryView']>>(view => {
    for (const held of [s0, s1, s2, s3, s4, s5, s6, s7]) if (held.enabled) held.onSummaryView(view);
  }, [s0, s1, s2, s3, s4, s5, s6, s7]);
  const summaries: (ChartSummary | undefined)[] = slots.map(held => held.enabled && tag ? held.lineOf(tag) : undefined);
  const error = slots.find(held => held.error)?.error;
  return { summaries, onSummaryView, error, retry: () => slots.forEach(held => held.retry()), owner: slots[0].owner };
}
