import type { Gate } from '../../dashboard/regionGate';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useEffect, useLayoutEffect, useMemo,  useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { easeTau } from '../../charts/easing';
import { jumpItem } from '../../charts/jump';
import { liveKey } from '../../charts/live';
import type { Live, LiveValues } from '../../charts/live';
import { scheduler } from '../../charts/scheduler';
import type { FrameTask } from '../../charts/scheduler';
import { runAt } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatAmount } from '../../charts/valueAxis';
import { movingRowClass, useRowMotion } from '../../charts/useRowMotion';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate, useHeldEdges } from '../../dashboard/regionGate';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame, useShownHour } from '../../dashboard/widgetPerf.ts';
import { OPTIONS } from './definition';
import ProductionDrill from './ProductionDrill';
import {
  bandScale, efficiencyFraction, efficiencyPercent, factoryScale, historyLanes, iconImage, laneStatus, lineKeyAt, lineValues, liveTotalText, organisationOf, organisationText,
  productionLineKey, productionLineLabel, totalText, validateAtReply, validateBreakdownReply, validateProductionReply, variantName, variantOf, weeklyRaw, weeklyText,
} from './productionData.ts';
import './productionLines.css';
import type { LineNow, LineOrder, ProductionLineBreakdownReply, ProductionLinesAtReply, ProductionLinesReply } from '../../generated/dashboard';
import { bucketEdges, bucketRead, UNRECORDED_MESSAGE, type BoundWindow } from '../../dashboard/regionTime.ts';
import type { CountryRef } from '../../generated/dashboard';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { ShownHour } from '../../dashboard/widgetPerf';

const NO_WARNINGS: readonly string[] = [];
const NO_CHANGES: readonly number[] = [];
type Part = 'efficiency' | 'output';

type Owner = { country: CountryRef; within?: BoundWindow[] };

export function useProductionLines(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, owner: Owner | null = null) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const asked = owner ?? (tag ? { country: countryFromKey(tag) } : null);
  const request = track && asked ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'production-lines', params: { branchId: track.branch.id, ...asked } }) : null;
  return { track, tag, ...useProduct<ProductionLinesReply>(runtime.client, request, active, validateProductionReply, track?.source) };
}

function useLineBreakdown(runtime: WidgetRuntime, widget: WidgetSpec, gate: Pick<Gate, 'recorded'>, owner: Owner | null, key: string | null, group: string, hour: number | null, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const read = gate.recorded(hour);
  const request = track && owner && key && read !== null && hour !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'production-line-breakdown',
    params: { branchId: track.branch.id, ...owner, key, group, hour: read } }) : null;
  return useProduct<ProductionLineBreakdownReply>(runtime.client, request, active, validateBreakdownReply, track?.source);
}

function useHeldOwner(gate: Pick<Gate, 'country' | 'span'>): Owner | null {
  const history = gate.country?.history ?? null;
  return useMemo(() => history ? { country: countryFromKey(history.key), within: history.spans.flatMap(span => gate.span(span.firstHour, span.lastHour) ?? []) } : null,
    [history, gate.span]);
}

export default function ProductionLinesWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const pinned = options.show === 'efficiency' || options.show === 'output';
  const lines = useProductionLines(widget, runtime, active && !pinned);
  const { track, tag, data, retry } = lines;
  const error = useHeldFailure(lines.error, data !== undefined, retry);
  const live = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const order: LineOrder = options.order === 'factories' || options.order === 'output' ? options.order : 'game';
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'production-lines-at', live && !pinned && tag ? { country: countryFromKey(tag), ...order === 'game' ? {} : { order } } : null,
    { active, validate: validateAtReply });
  const reportShown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  useShownEachFrame(runtime.clock, () => live ? shownAt.current() : Math.floor((track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour) ?? 0), live ? head.live : undefined);
  const motion = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Production line actions');
  const [detail, setDetail] = useState<number | null>(null);
  const [runEdges, holdRuns] = useHeldEdges();
  const changes = useMemo(() => track && detail ? [...new Set([...bucketEdges(track, detail), ...runEdges])].sort((a, b) => a - b) : NO_CHANGES,
    [track?.branch.firstHour, track?.branch.lastHour, detail, runEdges]);
  const savedPick = options.productionCountry;
  const drillNeeds = useMemo(() => ({ changes, holders: true, ...(pinned ? { country: { stored: savedPick } } : {}) }), [changes, pinned, savedPick]);
  const drillControl = useDrill(runtime, useRegion(widget), pinned ? options.show as string : null, undefined, undefined, drillNeeds);
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const owner = useHeldOwner(drillControl.gate);
  const drillLines = useProductionLines(widget, runtime, active && !!owner, owner);
  const drillError = useHeldFailure(drillLines.error, drillLines.data !== undefined, drillLines.retry);
  const drillDetail = drillLines.data?.detailHours ?? null;
  useEffect(() => { if (drillDetail !== null) setDetail(drillDetail); }, [drillDetail]);
  const [opened, setOpened] = useState<{ key: string; part: Part } | null>(null);
  const [factor, setFactor] = useState<string | null>(null);
  useEffect(() => { dispatch({ type: 'close' }); setOpened(null); }, [track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const extent: [number, number] = track ? [track.branch.firstHour, track.branch.lastHour] : [0, 1];
  const headHour = () => Math.floor((track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour) ?? extent[1]);
  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const shown: [number, number] = widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : extent;
  const lineTag = pinned ? drillControl.gate.country?.holder?.tag : tag && isSlotKey(tag) ? drillLines.data?.tag : tag;
  const pinnedKey = pinned && track ? productionLineKey(track.source.metadata.campaignKey, track.branch.id, options.line) : null;
  const key = pinned ? pinnedKey : opened?.key ?? null;
  const part: Part = (pinned ? options.show : opened?.part) === 'output' ? 'output' : 'efficiency';
  const group = factor ?? (pinned && typeof options.factor === 'string' && options.factor ? options.factor : part);
  const breakdownHour = drillControl.hour;
  const bucketHour = detail === null ? null : bucketRead(track, breakdownHour, detail, owner?.within);
  const breakdown = useLineBreakdown(runtime, widget, drillControl.gate, owner, key, group,
    bucketHour, active && (pinned || drill.breakdown !== null));
  useEffect(() => holdRuns(breakdown.data ? new Map([['runs', breakdown.data.runs]]) : null), [breakdown.data, holdRuns]);
  const warnings = useMemo(() => [...new Set([...data?.warnings ?? NO_WARNINGS, ...head.data?.warnings ?? NO_WARNINGS, ...breakdown.data?.warnings ?? NO_WARNINGS,
    ...breakdown.data?.runs.flatMap(run => run.warnings) ?? NO_WARNINGS])], [data, head.data, breakdown.data]);
  useDiagnostics(onDiagnostics, warnings);
  const slots = useSlotTags(runtime.client, track, active && !!tag && isSlotKey(tag));

  const open = (handleOrKey: string, nextPart: Part, hour: number, isKey = false) => {
    const lineKey = isKey ? handleOrKey : lineKeyAt(data, handleOrKey, hour);
    if (!lineKey || !tag) return;
    setFactor(null);
    setOpened({ key: lineKey, part: nextPart });
    dispatch({ type: 'open', breakdown: nextPart, hour, follow: drillControl.followsAt(hour), country: { tag, origin: Math.floor(hour) } });
  };

  if (!track || !tag) return <WidgetState kind="empty" icon="grid" message="Load a campaign and choose a country to see its production lines." />;
  const drillView = !key ? pinned && !lineTag ? drillError ? <WidgetState kind="error" failure={drillError} onRetry={drillLines.retry} /> : <WidgetState kind="loading" label="Loading production lines…" /> : <p className="drill-empty">Choose a production line in the widget settings.</p>
    : detail === null ? drillError ? <WidgetState kind="error" failure={drillError} onRetry={drillLines.retry} /> : <WidgetState kind="loading" label="Loading production lines…" />
    : breakdown.error ? <WidgetState kind="error" what="the breakdown" failure={breakdown.error} onRetry={breakdown.retry} />
      : bucketHour === null ? <WidgetState kind="unavailable" message={UNRECORDED_MESSAGE} />
      : !breakdown.data ? <WidgetState kind="loading" label="Loading breakdown…" />
        : <ProductionDrill data={breakdown.data} selected={group} onFactor={setFactor} hour={breakdownHour} active={active} hoursPerSecond={motion}
          onPick={hour => dispatch({ type: 'pick', hour })} />;
  const drillBody = <><ShownHour clock={runtime.clock} at={key && detail !== null && bucketHour !== null && breakdown.data ? breakdownHour : null} />{drillView}</>;
  if (pinned) return <div ref={anchor} className="production-lines"><DrillHost drill={drillControl} runtime={runtime} mode="inline" anchor={anchor}
    title={part === 'output' ? 'Output breakdown' : 'Efficiency breakdown'} day={breakdownHour == null ? '' : dayLabel(breakdownHour)} canFollow main={null}>{drillBody}</DrillHost></div>;
  if (error && !live) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const lineIndex = data?.lines.findIndex(line => line.key === key) ?? -1;
  const lineLabel = data && lineIndex >= 0 ? productionLineLabel(data, data.lines[lineIndex], lineIndex, breakdownHour ?? extent[1]) : 'Production line';
  const main = live ? <div ref={head.root} className="production-live">
    {head.error ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
      : !head.data ? head.read === 'off' ? <WidgetState kind="outside" /> : <WidgetState kind="loading" label="Loading production lines…" />
        : <LiveTable reply={head.data} history={data} live={head.live} report={() => reportShown(shownAt.current())} more={options.columns === 'more'}
          glide={Math.round(3 * easeTau(tokens.motion, motion, tokens.reducedMotion))} onBreakdown={(handle, nextPart) => open(handle, nextPart, head.hour ?? headHour())} />}
  </div>
    : !data ? <WidgetState kind="loading" label="Loading production lines…" />
      : <History data={data} name={keyNameAt(slots, tag, shown[1])} extent={shown} showEnded={options.ended !== 'hidden'} active={active} hoursPerSecond={motion}
        onMenu={(hour, x, y, lineKey) => menu.open(x, y, [jumpItem(runtime, track.id, hour), ...lineKey ? [
          { label: 'Efficiency breakdown', run: () => open(lineKey, 'efficiency', hour, true) },
          { label: 'Output breakdown', run: () => open(lineKey, 'output', hour, true) }] : []])} />;
  return <div ref={anchor} className="production-lines">
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && !!opened} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={`${part === 'output' ? 'Output' : 'Efficiency'} · ${lineLabel}`} day={breakdownHour == null ? '' : dayLabel(breakdownHour)} canFollow main={main}
      onClose={() => { dispatch({ type: 'close' }); setOpened(null); }}>{drillBody}</DrillHost>
    {menu.menu}
  </div>;
}

function LiveTable({ reply, history, live, report, more, glide, onBreakdown }: {
  reply: ProductionLinesAtReply; history: ProductionLinesReply | undefined; live: Live<LiveValues<number>>; report: () => void; more: boolean; glide: number;
  onBreakdown: (handle: string, part: Part) => void;
}) {
  const scale = useMemo(() => factoryScale(history), [history]);
  const items = useMemo(() => reply.lines.map(line => ({ key: line.handle, item: line })), [reply]);
  const rows = useRowMotion<LineNow>(items, true);
  const table = useRef<HTMLDivElement>(null);
  const latest = useRef({ reply, scale, report });
  latest.current = { reply, scale, report };
  const repaint = useRef<() => void>(() => {});
  useLayoutEffect(() => {
    let painted: LiveValues<number> | null = null;
    const paint = (force: boolean) => {
      const values = live.read();
      latest.current.report();
      if (!force && values === painted) return;
      painted = values;
      const { reply: now, scale: most } = latest.current;
      const byHandle = new Map(now.lines.map(line => [line.handle, line]));
      const cells = table.current?.querySelectorAll<HTMLElement>('[data-paint]') ?? [];
      let shared = most;
      for (const line of now.lines) {
        const read = lineValues(values, line.handle);
        shared = Math.max(shared, read.active ?? 0, read.requested ?? 0);
      }
      const percent = (value: number | null) => value !== null && shared > 0 ? `${Math.min(100, value / shared * 100)}%` : '0%';
      for (const cell of cells) {
        const handle = cell.dataset.line ?? '';
        const read = lineValues(values, handle);
        const why = (column: string) => values.missing(liveKey(handle, column));
        const text = (value: string, title = '') => {
          if (cell.textContent !== value) cell.textContent = value;
          if (cell.title !== title) cell.title = title;
        };
        switch (cell.dataset.paint) {
          case 'active': text(read.active === null ? '—' : read.active.toString(), read.active === null ? why('active') : ''); break;
          case 'requested': text(read.requested === null ? ' / —' : ` / ${read.requested}`); break;
          case 'activeBar': cell.style.width = percent(read.active); break;
          case 'requestedBar': cell.style.left = percent(read.requested); break;
          case 'efficiency': text(read.efficiency === null ? '—' : `${efficiencyPercent(read.efficiency)}%`, read.efficiency === null ? why('efficiency') : ''); break;
          case 'efficiencyBar': cell.style.width = read.efficiency === null ? '0%' : `${efficiencyFraction(read.efficiency) * 100}%`; break;
          case 'weekly': {
            const weekly = weeklyRaw(read.speed, read.cost);
            text(weekly === null ? '—' : weeklyText(weekly, now.decimals), read.speed === null ? why('speed') : read.cost === null ? why('cost') : '');
            break;
          }
          case 'output': {
            const line = byHandle.get(handle);
            text(liveTotalText(line?.output ?? null, read.output, now.outputMissing), now.outputMissing ?? (read.output === null && line?.output ? why('output')
              : line?.output && !line.output.wholeLife ? `Since ${dayLabel(line.output.sinceHour)}` : ''));
            break;
          }
          case 'cost': text(read.cost === null ? '—' : formatAmount(read.cost, now.decimals), read.cost === null ? why('cost') : ''); break;
        }
      }
    };
    repaint.current = () => paint(true);
    const task: FrameTask = { frame: () => { paint(false); return false; } };
    const remove = scheduler.add(task);
    const stop = live.subscribe(() => scheduler.wake(task));
    paint(true);
    return () => { remove(); stop(); repaint.current = () => {}; };
  }, [live]);
  useLayoutEffect(() => repaint.current());
  if (reply.missing) return <WidgetState kind="unavailable" message={reply.missing} />;
  if (!items.length && !rows.length) return <WidgetState kind="empty" message="No production lines running at this time." />;
  const style = { '--row-glide': `${glide}ms`,
    '--production-columns': more ? 'minmax(24px, .3fr) minmax(170px, 2.2fr) 1fr 1fr .8fr .8fr .7fr 1.1fr' : 'minmax(170px, 2.4fr) 1.1fr 1.1fr .9fr .9fr',
    height: `calc(var(--production-head) + ${rows.filter(row => row.phase !== 'leaving').length} * var(--production-row))` } as CSSProperties;
  return <div ref={table} className="production-table" role="list" aria-label="Production lines" style={style}>
    <div className="production-head" aria-hidden="true">
      {more && <span>#</span>}<span>Equipment</span><span>Factories</span><span>Efficiency</span><span className="is-number">Output / week</span><span className="is-number">Total</span>
      {more && <><span className="is-number">IC / unit</span><span>MIO</span></>}
    </div>
    {rows.map(row => {
      const line = row.item;
      const variant = variantOf(reply.variants, line.variant);
      const name = variantName(variant, line.variant);
      const art = iconImage(reply.icons, variant?.icon ?? null);
      const organisation = organisationOf(reply.organisations, line.mio);
      const orgArt = iconImage(reply.icons, organisation?.icon ?? null);
      const leaving = row.phase === 'leaving';
      const muted = reply.outputMissing || !line.output ? ' is-muted' : '';
      return <div key={row.key} role="listitem" className={`production-row ${movingRowClass(row)}`} aria-label={name}
        title={variant?.nameUnknown ?? undefined} style={{ transform: `translateY(calc(var(--production-head) + ${row.slot} * var(--production-row)))` }}>
        {more && <span className="production-number is-muted">{line.priority + 1}</span>}
        <span className="production-equipment">
          {art ? <img className="production-art" src={art} alt="" /> : <ArtFallback className="production-art" icon="hammer" />}
          <span><b>{name}</b>{variant?.typeName && variant.typeName !== name && <small>{variant.typeName}</small>}</span>
        </span>
        <span className="production-cell"><span><b className="production-number" data-paint="active" data-line={line.handle} /><small data-paint="requested" data-line={line.handle} /></span>
          <span className="production-meter" aria-hidden="true"><i data-paint="activeBar" data-line={line.handle} /><em data-paint="requestedBar" data-line={line.handle} /></span></span>
        <span className="production-cell">
          <button type="button" className="production-factor production-number" disabled={leaving} aria-label={`${name} efficiency breakdown`}
            onClick={() => onBreakdown(line.handle, 'efficiency')} data-paint="efficiency" data-line={line.handle} />
          <span className="production-meter" aria-hidden="true"><i data-paint="efficiencyBar" data-line={line.handle} /></span></span>
        <span className="production-number is-number">
          <button type="button" className="production-factor" disabled={leaving} aria-label={`${name} output breakdown`}
            onClick={() => onBreakdown(line.handle, 'output')} data-paint="weekly" data-line={line.handle} /></span>
        <span className={`production-number is-number${muted}`} data-paint="output" data-line={line.handle} />
        {more && <><span className="production-number is-number" data-paint="cost" data-line={line.handle} />
          <span className="production-organisation">{orgArt ? <img src={orgArt} alt="" /> : line.mio !== '0' && <ArtFallback icon="factory" />}
            <small>{organisationText(organisation, line.mio)}</small></span></>}
      </div>;
    })}
  </div>;
}

function History({ data, name, extent, showEnded, active, hoursPerSecond, onMenu }: {
  data: ProductionLinesReply; name: string | null; extent: [number, number]; showEnded: boolean; active: boolean; hoursPerSecond: number | undefined;
  onMenu: (hour: number, x: number, y: number, key?: string) => void;
}) {
  const height = useViewportHeight();
  const until = extent[1];
  const lanes = useMemo(() => historyLanes(data, until, showEnded, dayLabel), [data, until, showEnded]);
  const most = useMemo(() => bandScale(data), [data]);
  const byKey = useMemo(() => new Map(data.lines.map(line => [line.key, line])), [data]);
  const unobserved = useMemo(() => {
    const spans: { firstHour: number; lastHour: number }[] = [];
    let from = extent[0];
    for (const held of data.lineHours) {
      if (held.firstHour > from) spans.push({ firstHour: from, lastHour: Math.min(held.firstHour - 1, extent[1]) });
      from = Math.max(from, held.lastHour + 1);
    }
    if (from <= extent[1]) spans.push({ firstHour: from, lastHour: extent[1] });
    return spans;
  }, [data, extent]);
  if (!lanes.length) return <WidgetState kind={data.lineHours.length ? 'empty' : 'unavailable'}
    message={data.lineHours.length ? 'No production lines recorded in this time.' : name === null ? "Production lines weren't recorded." : `${name}'s production lines weren't recorded.`} />;
  return <div className="production-history">
    <Timeline label={name === null ? 'Production lines' : `Production lines of ${name}`} extent={extent} playhead={null} lanes={lanes} showIcons={false} showNames={false} bandMax={most}
      unobserved={unobserved} stickyAxis={false} fadePresentation truncateLabels={false} holderIconAlign="top" hoursPerSecond={hoursPerSecond} active={active} revision={data}
      tooltip={(lane, _span, hour, full) => {
        const line = byKey.get(lane.id);
        if (!line) return null;
        const step = runAt(line.steps, hour);
        const status = laneStatus(line, hour, dayLabel);
        const variant = step ? variantOf(data.variants, step.variant) : null;
        const forms = [{ title: step ? variantName(variant, step.variant) : lane.identity?.label ?? line.key, subtitle: dayLabel(hour), rows: [
          { label: 'Status', value: status.text },
          ...step ? [{ label: 'Factories', value: `${step.activeFactories} of ${step.requestedFactories}` }] : [],
          ...line.finalOutput ? [{ label: 'Units made', value: totalText(line.finalOutput) }] : [],
          ...line.beforeRecording ? [{ label: 'Began before the recording', tone: 'muted' as const }] : [],
        ] }];
        return full ? forms[0] : fitTooltip(forms, height, { full: forms[0] });
      }}
      onContextMenu={(hour, x, y, lane) => onMenu(hour, x, y, lane?.id)} />
  </div>;
}
