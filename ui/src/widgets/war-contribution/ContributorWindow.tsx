import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { drawingSurface, sizeCanvas, watchCanvasSize } from '../../charts/surface.ts';
import { canvasMeasureText } from '../../charts/textWidth.ts';
import { hideTooltip, showTooltip } from '../../charts/tooltip.ts';
import { CountryFlag } from '../../components/CountryFlag.tsx';
import type { WidgetRuntime } from '../../dashboard/definitions.ts';
import type { BoundTrack } from '../../dashboard/model.ts';
import { loadTransientProduct } from '../../dashboard/products.ts';
import { asFailure, failureText } from '../../dashboard/queryError.ts';
import type { ProductRequest } from '../../dashboard/request.ts';
import { drawSeriesMissingStrip } from '../../draw/kit.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { useFamilyTheme } from '../../families/host/theme.ts';
import { resolveTooltipEntryContent } from '../../families/host/tooltips.ts';
import { countryTagKey, useIdentities, useIdentityScope, type CountryIdentityResult } from '../../families/identity/react.ts';
import { countryHolderKey } from '../../families/identity/types.ts';
import { deriveTimeChartTooltip, hitTestTimeChart, parseChartInspect } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart, type TimeChartLayout } from '../../families/time-chart/layout.ts';
import { drawTimeChart } from '../../families/time-chart/painter.ts';
import { hasMeasuredValue } from '../../families/value.ts';
import { formatDateHover } from '../../families/valueFormat.ts';
import type { WarContributionHistoryReply, WarHistoryContributor, WarMeasureKey } from '../../generated/dashboard';
import { HISTORY_SCALES, detailAt, historyChart, memberAt, readingText } from './contributorHistory.ts';
import { MEASURE_LIST } from './definition.ts';

export interface OpenedContributor {
  readonly request: ProductRequest<'war-contribution-history'>;
  readonly warName: string;
  readonly at: number;
  readonly tag: string | null;
  readonly side: number;
  readonly measure: WarMeasureKey;
}
const keyAt = (person: WarHistoryContributor, hour: number) => {
  const member = memberAt(person, hour);
  return member ? countryTagKey(member[3]) : countryHolderKey(person.slot, person.id);
};
function countryName(identity: CountryIdentityResult, tag: string) {
  const field = identity.kind === 'known' ? identity.name : identity;
  const fallback = identity.kind === 'known' && identity.tag.kind === 'known' ? identity.tag.value : tag;
  return { label: field.kind === 'known' ? field.value : fallback, reason: field.kind === 'failed' ? field.failure.message : 'reason' in field ? field.reason : undefined };
}
type Inspection = { opened: OpenedContributor; hour: number; subjects: readonly WarHistoryContributor[]; focused?: WarHistoryContributor };
function HistoryCanvas({ model, hour, first, inspect, theme }: { readonly model: ReturnType<typeof historyChart>; readonly hour: number; readonly first: number; readonly inspect: (hour: number) => void; readonly theme: FamilyTheme }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const lastVisit = useRef<[string, number, number] | null>(null);
  const state = useRef<{ layout: TimeChartLayout | null; key: string | null; paint?: () => void }>({ layout: null, key: null });
  const tooltip = useId();
  useLayoutEffect(() => {
    const element = canvas.current;
    if (!element) return;
    if (element.closest('[role="dialog"]') === document.activeElement) element.focus({ preventScroll: true });
    const { canvas: surface, context } = drawingSurface(element);
    if (!context) return;
    const measure = canvasMeasureText(context);
    const marks = model.series.flatMap(({ points, missingSpans, color }) => [
      ...points.filter((pt, i) => pt.value !== null && points[i - 1]?.value == null && points[i + 1]?.value == null).map(pt => ({ ...pt, color })),
      ...missingSpans.filter(span => span.firstHour === span.lastHour).map(span => ({ hour: span.firstHour, value: null, color })),
    ]);
    const layoutChart = () => {
      const { layoutWidth, layoutHeight, backing } = size.read();
      sizeCanvas(surface, backing.width, backing.height);
      const layout = layoutTimeChart(model, layoutWidth, layoutHeight, theme.font, measure);
      state.current.layout = layout;
      state.current.paint = () => {
        drawTimeChart(context, model, layout, theme, state.current.key, backing.scaleX, null, undefined, { cacheBackdrop: true });
        for (const { hour, value, color } of marks) {
          const x = layout.toX(hour);
          if (value === null) drawSeriesMissingStrip(context, Math.min(x, layout.left + layout.plotWidth - 2), layout.baseY - 6, 2, color, theme, 5);
          else { context.beginPath(); context.arc(x, layout.toY(value), 2.5, 0, Math.PI * 2); context.fillStyle = color; context.fill(); }
        }
      };
      state.current.paint();
    };
    const size = watchCanvasSize(element, layoutChart);
    layoutChart();
    return () => { size.disconnect(); state.current = { layout: null, key: null }; hideTooltip(tooltip); };
  }, [model, theme, tooltip]);
  const visit = (key: string | null, x = 0, y = 0) => {
    lastVisit.current = key ? [key, x, y] : null;
    const { layout } = state.current;
    if (!layout) return;
    const hit = parseChartInspect(key);
    const at = hit?.kind === 'marker' ? layout.markers[hit.index]?.marker.hour ?? null
      : hit?.kind === 'plot' || hit?.kind === 'line' ? Math.floor(hit.hour) : null;
    if (at !== null && at < first) key = `plot:${first}`;
    if (key !== state.current.key) { state.current.key = key; state.current.paint?.(); }
    const entry = deriveTimeChartTooltip({ ...model, series: [], hoverDetail: at === null ? undefined : model.detailAt(Math.max(first, at)) }, layout, key);
    if (!entry) return hideTooltip(tooltip);
    inspect(Math.max(first, Math.min(model.hour, Math.floor(entry.hour))));
    showTooltip(tooltip, resolveTooltipEntryContent(entry), x, y);
  };
  useLayoutEffect(() => { if (lastVisit.current) visit(...lastVisit.current); }, [model, theme]);
  return <canvas ref={canvas} className="war-history-chart" role="img" tabIndex={0}
    aria-label={`${model.measureTitle} over the war. Use arrow keys to inspect the chart by hour.`}
    onPointerLeave={() => visit(null)} onBlur={() => visit(null)}
    onPointerMove={event => {
      const layout = state.current.layout;
      const rect = event.currentTarget.getBoundingClientRect();
      visit(layout && hitTestTimeChart(model, layout, [event.clientX - rect.left, event.clientY - rect.top]), event.clientX, event.clientY);
    }}
    onKeyDown={event => {
      const next = { ArrowLeft: hour - 1, ArrowRight: hour + 1, Home: first, End: model.hour }[event.key];
      if (next === undefined) return;
      event.preventDefault();
      const at = Math.min(model.hour, Math.max(first, next));
      const rect = event.currentTarget.getBoundingClientRect();
      visit(`plot:${at}`, rect.left + (state.current.layout?.toX(at) ?? 0), rect.top + 24);
    }} />;
}

function HistoryBody({ history, opened, identities, setInspection }: { readonly history: WarContributionHistoryReply; readonly opened: OpenedContributor; readonly identities: ReturnType<typeof useIdentities>; readonly setInspection: (inspection: Inspection) => void }) {
  const { theme } = useFamilyTheme();
  const [measure, setMeasure] = useState(opened.measure);
  const [scale, setScale] = useState(1);
  const [compare, setCompare] = useState(opened.tag === null);
  const [hour, setHour] = useState(opened.at);
  const index = history.measureKeys.indexOf(measure);
  const side = useMemo(() => history.contributors.filter(person => memberAt(person, opened.at)?.[2] === opened.side), [history, opened]);
  const rank = useMemo(() => side.flatMap(person => {
    const value = detailAt(history, person, index, opened.at)[0]!;
    return hasMeasuredValue(value) ? [{ person, magnitude: value.value < 0n ? -value.value : value.value }] : [];
  }).sort((a, b) => a.magnitude === b.magnitude ? a.person.slot - b.person.slot : a.magnitude > b.magnitude ? -1 : 1), [side, history, index, opened.at]);
  const focused = opened.tag === null ? rank[0]?.person ?? side[0] : side.find(person => memberAt(person, opened.at)?.[3] === opened.tag);
  const people = useMemo(() => compare && rank.length ? rank.slice(0, 5).map(({ person }) => person) : focused ? [focused] : [], [compare, rank, focused]);
  const base = useMemo(() => theme ? historyChart(history, people, measure, scale, theme.surface) : null, [history, people, measure, scale, theme]);
  const hidden = compare ? side.length - people.length : 0;
  const tag = focused && memberAt(focused, hour)?.[3];
  const openingTag = focused && memberAt(focused, opened.at)?.[3];
  const { resolve, flagFor } = identities;
  useLayoutEffect(() => setInspection({ opened, hour, focused, subjects: focused && !people.includes(focused) ? [...people, focused] : people }),
    [opened, hour, focused, people, setInspection]);
  const nameAt = useCallback((person: WarHistoryContributor, at: number) => countryName(resolve(keyAt(person, at), at), memberAt(person, at)?.[3] ?? memberAt(person, opened.at)![3]), [resolve, opened.at]);
  const labelsKey = JSON.stringify(people.map(person => nameAt(person, hour).label));
  const model = useMemo(() => {
    if (!base) return null;
    const labels = JSON.parse(labelsKey) as string[];
    return { ...base, series: base.series.map((line, i) => ({ ...line, name: labels[i]! })), legend: base.legend?.map((entry, i) => ({ ...entry, label: labels[i]! })),
      markers: base.markers.map(mark => { const name = countryName(resolve(countryTagKey(mark.tag), mark.hour), mark.tag); return { ...mark, title: name.label + mark.title.slice(mark.tag.length), detail: name.reason }; }),
      detailAt: (at: number) => base.detailAt(at, nameAt) };
  }, [base, labelsKey, nameAt, resolve]);
  const identity = focused && resolve(keyAt(focused, hour), hour);
  const { label: name, reason } = identity ? countryName(identity, (tag ?? openingTag)!) : { label: undefined, reason: undefined };
  const flag = identity && identity.kind === 'known' && identity.flagChoice.kind === 'known' ? flagFor(identity.flagChoice.value) : null;
  const unread = history.gaps.find(gap => gap.firstHour <= hour && hour <= gap.lastHour)?.reason ?? 'The contributor’s history was not recorded for this selection.';
  return <>
    <div className="war-history-options">
      <label>Measure <select value={measure} onChange={event => setMeasure(event.currentTarget.value as WarMeasureKey)}>
        {MEASURE_LIST.map(item => <option key={item.key} value={item.key}>{item.label}</option>)}
      </select></label>
      <label>Scale <select value={scale} onChange={event => setScale(Number(event.currentTarget.value))}>
        {HISTORY_SCALES.map((name, i) => <option key={name} value={i}>{name}</option>)}
      </select></label>
      {opened.tag !== null && <button type="button" aria-pressed={compare} onClick={() => setCompare(value => !value)}>Compare side</button>}
      {hidden > 0 && <span>{hidden} {hidden === 1 ? 'contributor' : 'contributors'} not shown</span>}
    </div>
    {model && theme && <HistoryCanvas model={model} hour={hour} first={history.war.firstHour} inspect={setHour} theme={theme} />}
    {focused ? <table className="war-history-readings">
      <caption aria-live="polite" data-tooltip={reason}>{tag && <CountryFlag image={flag?.image} tag={tag} name={name} decorative />}
        {name} · {formatDateHover(hour)}<span className="sr-only"> {model?.measureTitle}. {model?.detailAt(hour).items.map(item => `${item.label} ${item.value}`).join('. ')}</span></caption>
      <thead><tr><th scope="col">Measure</th>{HISTORY_SCALES.map(label => <th key={label} scope="col">{label}</th>)}</tr></thead>
      <tbody>{MEASURE_LIST.map(({ key, label }) => <tr key={key}><th scope="row">{label}</th>
        {detailAt(history, focused, history.measureKeys.indexOf(key), hour).map((value, i) =>
          <td key={HISTORY_SCALES[i]}>{readingText(value, i)}</td>)}
      </tr>)}</tbody>
    </table> : <p role="status">{unread}</p>}
  </>;
}

export default function ContributorWindow({ opened, track, runtime, anchor, onClose }: {
  readonly opened: OpenedContributor;
  readonly track: BoundTrack;
  readonly runtime: WidgetRuntime;
  readonly anchor: RefObject<HTMLDivElement | null>;
  readonly onClose: () => void;
}) {
  const [result, setResult] = useState<WarContributionHistoryReply | string>();
  const [inspection, setInspection] = useState<Inspection>();
  const [request] = useState(opened.request);
  useEffect(() => {
    const controller = new AbortController();
    void Promise.resolve().then(() => {
      controller.signal.throwIfAborted();
      return loadTransientProduct<WarContributionHistoryReply>(runtime.client, request, controller.signal);
    }).then(
      data => { if (!controller.signal.aborted) setResult(data); },
      error => { if (!controller.signal.aborted) setResult(failureText(asFailure(error), 'Contribution history was not recorded')); },
    );
    return () => controller.abort();
  }, [runtime.client, request]);
  const scope = useIdentityScope(runtime.client, track.source, track.branch.id);
  const shown = inspection?.opened === opened ? inspection : undefined;
  const at = shown?.hour ?? opened.at;
  const titleKey = shown?.focused ? keyAt(shown.focused, at) : opened.tag ? countryTagKey(opened.tag) : null;
  const asks = shown?.subjects.map(person => ({ key: keyAt(person, at), hour: at })) ?? [];
  if (titleKey && !shown?.focused) asks.push({ key: titleKey, hour: opened.at });
  const identities = useIdentities(runtime.client, track.source, scope, asks, { needFlags: true, flagSize: 'small', branchBounds: [track.branch.firstHour, track.branch.lastHour] });
  const title = opened.tag && titleKey ? countryName(identities.resolve(titleKey, shown?.focused ? at : opened.at), opened.tag) : undefined;
  return <DetailWindowHost open subject={String(request.params.warId)} hour={opened.at} anchor={anchor} returnFocus=".fh-body[tabindex], .fh-sr-only[tabindex='0'], .fh-sr-only [tabindex='0']"
    className="war-history" settled={result !== undefined} onClose={onClose}
    title={<>{opened.tag && <span data-tooltip={title?.reason}>{title?.label ?? opened.tag} · </span>}Contributor history · {opened.warName}</>}>
    {typeof result === 'object' ? <HistoryBody key={`${opened.side}:${opened.tag}:${opened.measure}`} history={result} opened={opened} identities={identities} setInspection={setInspection} />
      : <p role="status">{result ?? 'Loading wars…'}</p>}
  </DetailWindowHost>;
}
