import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type RefObject } from 'react';
import { drawingSurface, sizeCanvas, watchCanvasSize } from '../../charts/surface.ts';
import { canvasMeasureText } from '../../charts/textWidth.ts';
import { hideTooltip, showTooltip } from '../../charts/tooltip.ts';
import { CountryFlag } from '../../components/CountryFlag.tsx';
import type { WidgetRuntime } from '../../dashboard/definitions.ts';
import type { BoundTrack } from '../../dashboard/model.ts';
import { loadTransientProduct } from '../../dashboard/products.ts';
import { asFailure, failureText } from '../../dashboard/queryError.ts';
import type { BoundHour } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import type { FamilyTheme } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import { useFamilyTheme } from '../../families/host/theme.ts';
import { resolveTooltipEntryContent } from '../../families/host/tooltips.ts';
import { countryTagKey, useIdentity, useIdentityScope } from '../../families/identity/react.ts';
import { deriveTimeChartTooltip, hitTestTimeChart } from '../../families/time-chart/hitTest.ts';
import { layoutTimeChart, type TimeChartLayout } from '../../families/time-chart/layout.ts';
import type { TimeChartModel } from '../../families/time-chart/model.ts';
import { drawTimeChart } from '../../families/time-chart/painter.ts';
import { hasMeasuredValue } from '../../families/value.ts';
import { formatDateHover } from '../../families/valueFormat.ts';
import type { WarContributionHistoryReply, WarMeasureKey } from '../../generated/dashboard';
import { HISTORY_SCALES, historyChart, memberAt, readContribution, readingText } from './contributorHistory.ts';
import { MEASURE_LIST } from './definition.ts';

export interface OpenedContributor {
  readonly warId: string;
  readonly viewpoint: string;
  readonly warName: string;
  readonly through: BoundHour;
  readonly at: number;
  readonly tag: string | null;
  readonly side: number;
  readonly measure: WarMeasureKey;
}

function HistoryCanvas({ model, hour, first, inspect, theme }: { readonly model: TimeChartModel; readonly hour: number; readonly first: number; readonly inspect: (hour: number) => void; readonly theme: FamilyTheme }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const state = useRef<{ layout: TimeChartLayout | null; key: string | null; paint?: () => void }>({ layout: null, key: null });
  const tooltip = useId();
  useLayoutEffect(() => {
    if (canvas.current?.closest('[role="dialog"]') === document.activeElement) canvas.current?.focus({ preventScroll: true });
  }, []);
  useLayoutEffect(() => {
    if (!canvas.current) return;
    const surface = drawingSurface(canvas.current);
    const context = surface.context;
    if (!context) return;
    const measure = canvasMeasureText(context);
    const layoutChart = () => {
      const { layoutWidth, layoutHeight, backing } = size.read();
      sizeCanvas(surface.canvas, backing.width, backing.height);
      const layout = layoutTimeChart(model, layoutWidth, layoutHeight, theme.font, measure);
      state.current.layout = layout;
      state.current.paint = () => drawTimeChart(context, model, layout, theme, state.current.key, backing.scaleX, null, undefined, { cacheBackdrop: true });
      state.current.paint();
    };
    const size = watchCanvasSize(canvas.current, layoutChart);
    layoutChart();
    return () => { size.disconnect(); state.current = { layout: null, key: null }; hideTooltip(tooltip); };
  }, [model, theme, tooltip]);
  const select = (key: string | null) => {
    state.current.key = key;
    state.current.paint?.();
  };
  return <canvas ref={canvas} className="war-history-chart" role="img" tabIndex={0}
    aria-label={`${model.measureTitle} over the war. Use arrow keys to inspect the chart by hour.`}
    onPointerLeave={() => { hideTooltip(tooltip); select(null); }}
    onPointerMove={event => {
      const layout = state.current.layout;
      if (!layout) return;
      const rect = event.currentTarget.getBoundingClientRect();
      const key = hitTestTimeChart(model, layout, [event.clientX - rect.left, event.clientY - rect.top]);
      if (key !== state.current.key) select(key);
      const entry = key && deriveTimeChartTooltip(model, layout, key);
      const content = entry && resolveTooltipEntryContent(entry);
      if (entry) inspect(Math.max(first, Math.min(model.hour, Math.floor(entry.hour))));
      const note = content && [...new Set(content.rows.flatMap(row => row.valueState?.reason ? [row.valueState.reason] : []))].join('. ');
      if (content) showTooltip(tooltip, { ...content, note: note || content.note }, event.clientX, event.clientY); else hideTooltip(tooltip);
    }}
    onKeyDown={event => {
      const next = { ArrowLeft: hour - 1, ArrowRight: hour + 1, Home: first, End: model.hour }[event.key];
      if (next === undefined) return;
      event.preventDefault();
      const at = Math.min(model.hour, Math.max(first, next));
      inspect(at);
      select(`plot:${at}`);
    }} />;
}

function HistoryBody({ history, opened, track, runtime }: { readonly history: WarContributionHistoryReply; readonly opened: OpenedContributor; readonly track: BoundTrack; readonly runtime: WidgetRuntime }) {
  const { theme } = useFamilyTheme();
  const [measure, setMeasure] = useState(opened.measure);
  const [scale, setScale] = useState(1);
  const [compare, setCompare] = useState(opened.tag === null);
  const [hour, setHour] = useState(opened.at);
  const index = history.measureKeys.indexOf(measure);
  const side = useMemo(() => history.contributors.filter(person => memberAt(person, opened.at)?.[2] === opened.side), [history, opened]);
  const rank = useMemo(() => side.flatMap(person => {
    const value = readContribution(history, person, index, opened.at)[0]!;
    return hasMeasuredValue(value) ? [{ person, magnitude: value.value < 0n ? -value.value : value.value }] : [];
  }).sort((a, b) => a.magnitude === b.magnitude ? a.person.slot - b.person.slot : a.magnitude > b.magnitude ? -1 : 1), [side, history, index, opened.at]);
  const focused = opened.tag === null ? rank[0]?.person ?? side[0] : side.find(person => memberAt(person, opened.at)?.[3] === opened.tag);
  const model = useMemo(() => theme ? historyChart(history, compare && rank.length ? rank.slice(0, 5).map(({ person }) => person) : focused ? [focused] : [], measure, scale, theme.surface) : null,
    [history, compare, rank, focused, measure, scale, theme]);
  const hidden = compare ? side.length - (model?.series.length ?? 0) : 0;
  const tag = focused && memberAt(focused, hour)?.[3];
  const scope = useIdentityScope(runtime.client, track.source, track.branch.id);
  const { result: identity, flag, flagArt } = useIdentity(runtime.client, track.source, scope, tag ? countryTagKey(tag) : null, hour,
    { needFlag: true, flagSize: 'small', branchBounds: [track.branch.firstHour, track.branch.lastHour] });
  const name = identity && identity.kind === 'known' && identity.name.kind === 'known' ? identity.name.value : tag ?? (focused && memberAt(focused, opened.at)?.[3]);
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
      <caption aria-live="polite">{tag && <CountryFlag image={flag?.image} tag={tag} name={name} pending={flagArt.status === 'pending'} decorative />}
        {name} · {formatDateHover(hour)}<span className="sr-only"> {MEASURE_LIST.find(item => item.key === measure)?.label}. {readingText(readContribution(history, focused, index, hour)[scale]!, scale)}</span></caption>
      <thead><tr><th scope="col">Measure</th>{HISTORY_SCALES.map(label => <th key={label} scope="col">{label}</th>)}</tr></thead>
      <tbody>{MEASURE_LIST.map(({ key, label }) => <tr key={key}><th scope="row">{label}</th>
        {readContribution(history, focused, history.measureKeys.indexOf(key), hour).map((value, i) =>
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
  const { client } = runtime;
  const [result, setResult] = useState<WarContributionHistoryReply | string>();
  const { warId, through, viewpoint } = opened;
  const { id: sourceId, revision } = track.source;
  const branchId = track.branch.id;
  useEffect(() => {
    const request = productRequest({ sourceId, revision, product: 'war-contribution-history', params: { branchId, viewpoint, warId, throughHour: through } });
    const controller = new AbortController();
    void Promise.resolve().then(() => {
      controller.signal.throwIfAborted();
      return loadTransientProduct<WarContributionHistoryReply>(client, request, controller.signal);
    }).then(
      data => { if (!controller.signal.aborted) setResult(data); },
      error => { if (!controller.signal.aborted) setResult(failureText(asFailure(error), 'Contribution history was not recorded')); },
    );
    return () => controller.abort();
  }, [client, sourceId, revision, branchId, viewpoint, warId, through]);
  return <DetailWindowHost open subject={warId} hour={opened.at} anchor={anchor} returnFocus=".fh-body[tabindex], .fh-sr-only[tabindex='0'], .fh-sr-only [tabindex='0']"
    className="war-history" settled={result !== undefined} onClose={onClose}
    title={`${opened.tag ? `${opened.tag} · ` : ''}Contributor history · ${opened.warName}`}>
    {typeof result === 'object' ? <HistoryBody key={`${opened.side}:${opened.tag}:${opened.measure}`} history={result} opened={opened} track={track} runtime={runtime} />
      : <p role="status">{result ?? 'Reading contributor history…'}</p>}
  </DetailWindowHost>;
}
