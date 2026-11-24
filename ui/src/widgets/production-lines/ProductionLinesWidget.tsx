import type { Gate } from '../../dashboard/regionGate';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import Timeline from '../../charts/timeline/Timeline';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps, WidgetRuntime } from '../../dashboard/definitions';
import { useHeldEdges } from '../../dashboard/regionGate';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { resolveWindow } from '../../dashboard/model';
import type { WidgetSpec } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf.ts';
import { ShownHour } from '../../dashboard/widgetPerf';
import { productRequest } from '../../dashboard/request.ts';
import { bucketEdges, bucketRead, UNRECORDED_MESSAGE, type BoundWindow } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { DetailWindowHost } from '../../families/host/DetailWindowHost.tsx';
import DeclaredTableWidget from '../../families/table/DeclaredTableWidget.tsx';
import type { CountryRef, ProductionLineBreakdownReply, ProductionLinesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import ProductionDrill from './ProductionDrill';
import {
  bandScale,
  historyLanes,
  laneStatus,
  lineKeyAt,
  productionLineKey,
  productionLineLabel,
  totalText,
  validateBreakdownReply,
  validateProductionReply,
  variantName,
  variantOf,
} from './productionData.ts';
import { productionLinesDeclaration } from './productionLinesProjection.ts';
import './productionLines.css';

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

export default function ProductionLinesWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const pinned = options.show === 'efficiency' || options.show === 'output';
  const live = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const lines = useProductionLines(widget, runtime, active && (!pinned && !live));
  const { track, tag, data, retry } = lines;
  const error = useHeldFailure(lines.error, data !== undefined, retry);
  useShownEachFrame(runtime.clock, () => Math.floor((track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour) ?? 0), undefined);
  const motion = usePlaybackMotion(runtime.clock);
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
  const warnings = useMemo(() => [...new Set([...data?.warnings ?? NO_WARNINGS, ...breakdown.data?.warnings ?? NO_WARNINGS,
    ...breakdown.data?.runs.flatMap(run => run.warnings) ?? NO_WARNINGS])], [data, breakdown.data]);
  useDiagnostics(onDiagnostics, live && !pinned ? null : warnings);
  const slots = useSlotTags(runtime.client, track, active && !live && !!tag && isSlotKey(tag));

  const open = (handleOrKey: string, nextPart: Part, hour: number, isKey = false) => {
    const lineKey = isKey ? handleOrKey : lineKeyAt(drillLines.data ?? data, handleOrKey, hour);
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

  const lineIndex = data?.lines.findIndex(line => line.key === key) ?? -1;
  const lineLabel = data && lineIndex >= 0 ? productionLineLabel(data, data.lines[lineIndex], lineIndex, breakdownHour ?? extent[1]) : 'Production line';

  if (live) {
    return <div ref={anchor} className="production-lines">
      <DeclaredTableWidget
        declaration={productionLinesDeclaration}
        {...props}
        onActivate={(handle, _meta, hour) => open(handle, 'efficiency', hour ?? headHour())}
      />
      <DetailWindowHost
        open={drill.breakdown !== null && !!opened}
        subject={opened?.key ?? null}
        hour={breakdownHour}
        title={`${part === 'output' ? 'Output' : 'Efficiency'} · ${lineLabel}`}
        day={breakdownHour == null ? '' : dayLabel(breakdownHour)}
        anchor={anchor}
        onClose={() => { dispatch({ type: 'close' }); setOpened(null); }}
      >
        {drillBody}
      </DetailWindowHost>
    </div>;
  }

  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const main = !data ? <WidgetState kind="loading" label="Loading production lines…" />
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
