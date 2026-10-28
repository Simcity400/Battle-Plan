import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import { useContextMenu } from '../../components/ContextMenu';
import DrillParts from '../../charts/drill/DrillParts';
import { dayLabel, drawLine, drawBands, drawPlayhead } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { nudgeApart } from '../../charts/palette';
import { compactStack, compactStackExtent, forEachCompactStackSegment } from '../../charts/stack';
import { heldStackSegment } from '../../charts/race/countryRace';
import { extentOf } from '../../charts/series';
import { drawApproximateSpans, drawLineSummary, extentOfSummary, summaryStack, summaryFits } from '../../charts/chartSummary';
import TimeChart from '../../charts/TimeChart';
import { pendingLayer } from '../../charts/prepared';
import { PreparedKey } from '../../charts/PreparedKey';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { formatDateTime } from '../../dashboard/calendar';
import { nameKey, useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { FAILED_FACT, MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import type { NameAt } from '../../dashboard/countryFacts';
import { useCountryFlagsAt, useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import { usePreparedWindow } from '../../dashboard/preparedSeries';
import { useParityRows } from '../../dashboard/parityRows';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { parseColor, toHex } from '../../shell/appearance/color';
import { contributionPercent, countryKey, formatContribution, formatTension, formatTensionAxis, tensionModel, tensionReadPreparing } from './tensionData';
import { mergePreparedTension, validatePreparedTension } from './preparedTension';
import type { TensionModel } from './tensionData';
import { tensionCauseAt, tensionDrillAt, tensionDrillHistory, tensionEventKeys, tensionLookAt, tensionShareText, tensionVisibleParts } from './tensionDrill';

/** The full native country stack. Event final amounts stay separate because the
 * product does not record source current amounts or per-read source presence. */
export default function TensionDrill({ model: reached, runtime, track, active, options, extent, playhead, hour, selectedCause, onCause, onPick }: {
  model: TensionModel; runtime: WidgetRuntime; track: BoundTrack; active: boolean;
  options: Readonly<Record<string, OptionValue>>; extent: [number, number]; playhead: number | null; hour: number;
  selectedCause: string | null; onCause: (key: string | null) => void; onPick: (hour: number) => void;
}) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const [hoverHour,setHoverHour] = useState<number|null>(null);
  const prepared = usePreparedWindow(runtime.client,track.source,'world-tension',{ branchId: track.branch.id,share: false },extent,
    [hour,...(hoverHour === null ? [] : [hoverHour])],active,validatePreparedTension,mergePreparedTension);
  const model = useMemo(() => prepared.data ? tensionModel(prepared.data,reached.lineage,track.source.metadata.countries,
    new Map(prepared.data.catalogue.map(country => [countryKey(country),country.firstHour]))) : reached,[prepared.data,reached,track.source]);
  const keys = useMemo(() => [...model.raw.keys()], [model]);
  const at = tensionDrillAt(model, hour);
  const [visible, setVisible] = useState<readonly string[]>([]);
  const visibleSet = new Set(visible);
  const onVisible = useCallback((ids: readonly string[]) => setVisible(previous => previous.length === ids.length
    && previous.every((id, index) => id === ids[index]) ? previous : ids), []);
  const planned = useMemo(() => new Map(keys.flatMap(key => {
    const sample = tensionLookAt(model, key, hour, track.branch);
    return sample ? [[key, sample] as const] : [];
  })), [model, keys, hour, track.branch]);
  const sampleKey = JSON.stringify([...planned].map(([key, sample]) => [key, sample.tag, sample.hour, sample.stretch]));
  // Ordinary playback inside one sample grid does not re-run the palette.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const samples = useMemo(() => planned, [sampleKey]);
  const wanted = useMemo(() => [...samples.values()], [samples]);
  const maxHours = Math.max(MAX_NAME_HOURS, new Set(wanted.map(sample => sample.hour)).size);
  // Every drawn band needs its recorded style; only mounted rows need flags.
  const styles = useCountryFactsRead(runtime.client, track, wanted, active, STYLES, maxHours);
  const visibleWanted = tensionVisibleParts(at?.parts ?? [], visible).flatMap(part => {
    const sample = samples.get(part.key); return sample ? [sample] : [];
  });
  const flags = useCountryFlagsAt(runtime.client, track, visibleWanted, active, 'panel', maxHours);
  const colours = useMemo(() => {
    const known = [...samples].sort(([a], [b]) => a.localeCompare(b)).flatMap(([key, sample]) => {
      const style = styles.get(nameKey(sample.hour, sample.tag));
      return style && style !== FAILED_FACT && style.color
        ? [{ key, color: { r: style.color[0], g: style.color[1], b: style.color[2] } }] : [];
    });
    const raw = known.map(item => item.color);
    const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw, parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw;
    return new Map(known.map((item, index) => [item.key, toHex(adjusted[index])]));
  }, [samples, styles, tokens.countryColours, tokens.surface]);
  const countryLabel = (key: string) => {
    const sample = samples.get(key);
    const style = sample ? styles.get(nameKey(sample.hour, sample.tag)) : null;
    return style && style !== FAILED_FACT ? style.name ?? 'Country name unavailable' : 'Country name unavailable';
  };
  const layerColours = useMemo(() => keys.map(key => colours.get(key) ?? tokens.muted).concat(tokens.muted), [keys, colours, tokens.muted]);
  const history = useMemo(() => tensionDrillHistory(model, keys), [model, keys]);
  const stack = useMemo(() => compactStack(history.bands), [history]);
  const denseParts = useMemo(() => prepared.data && prepared.geometryReady && keys.every(key => prepared.data!.summaries[JSON.stringify(key)])
    ? keys.map(key => prepared.data!.summaries[JSON.stringify(key)]) : null,[prepared.data,prepared.geometryReady,keys]);
  const denseStack = useMemo(() => denseParts && prepared.data?.summaries.sum ? summaryStack(denseParts,prepared.data.summaries.sum) : null,[denseParts,prepared.data?.summaries.sum]);
  const [tooltipWanted, setTooltipWanted] = useState<readonly NameAt[]>([]);
  const asked = useRef('');
  const ask = (items: readonly NameAt[]) => {
    const key = JSON.stringify(items);
    if (key === asked.current) return;
    asked.current = key;
    queueMicrotask(() => setTooltipWanted(items));
  };
  useEffect(() => { asked.current = ''; setTooltipWanted([]); }, [active, model.reply, track.source.id, track.source.revision, track.branch.id]);
  const tooltipLabels = useCountryLabelsAt(runtime.client, track, tooltipWanted, active,
    Math.max(MAX_NAME_HOURS, new Set(tooltipWanted.map(item => item.hour)).size));
  const pickOwner = prepared.pickOwner;
  const playback = options.playback as PlaybackMode;
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const summary = prepared.data?.summaries.world;
      if (!prepared.geometryReady || !summary || !summaryFits(frame, summary, to)) return;
      if (!summary.runs || summary.approximate?.length) {
        if (denseStack) drawApproximateSpans(ctx,frame,denseStack.approximate,state.playhead,() => forEachCompactStackSegment(denseStack.model,frame.x[0],to,0,piece => {
          const segment=heldStackSegment(piece);
          drawBands(ctx,frame,chartTokens,segment.hours,segment.bands,layerColours,segment);
        },denseStack.sample));
        drawLineSummary(ctx,frame,chartTokens,summary,chartTokens.text,true,state.playhead,{ endDot: false });
        drawPlayhead(ctx,frame,chartTokens,hour); return;
      }
      forEachCompactStackSegment(stack, frame.x[0], to, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2, piece => {
        const segment = heldStackSegment(piece);
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, layerColours, segment);
      });
      drawLine(ctx, frame, chartTokens, history.world, chartTokens.text, { until: to, held: true });
      drawPlayhead(ctx, frame, chartTokens, hour);
    },
    extent: x => {
      if (!prepared.geometryReady) return null;
      if (!prepared.data?.summaries.world?.runs || prepared.data.summaries.world.approximate?.length) {
        const bands=denseStack ? compactStackExtent(denseStack.model,x[0],Math.min(x[1],until ?? x[1])) : null;
        const world=prepared.data?.summaries.world ? extentOfSummary(prepared.data.summaries.world) : null;
        return bands && world ? [Math.min(bands[0],world[0]),Math.max(bands[1],world[1])] : bands ?? world;
      }
      const to = Math.min(x[1], until ?? x[1]);
      const bands = compactStackExtent(stack, x[0], to);
      const world = extentOf(history.world, x[0], to);
      return bands && world ? [Math.min(bands[0], world[0]), Math.max(bands[1], world[1])] : bands ?? world;
    },
  }], [stack, history, layerColours, hour, until,prepared.data,prepared.geometryReady,denseStack]);
  const menu = useContextMenu('Country contribution actions');
  const eventKeys = useMemo(() => tensionEventKeys(model.reply.events), [model.reply.events]);
  const causeIndex = selectedCause == null ? -1 : eventKeys.indexOf(selectedCause);
  const cause = causeIndex < 0 ? null : model.reply.events[causeIndex];
  const causeRead = cause ? tensionCauseAt(cause, hour) : null;
  const exact = (raw: string) => formatContribution(model, raw, true, model.contributionDecimals);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root,() => ({ kind: 'chart',label: 'World tension breakdown',scope: 'displayed-hour',hour,
    complete: !!at || !!prepared.data?.exactHours.includes(hour),total: at?.parts.length ?? 0,rows: at?.parts ?? [],read: at?.read,remainder: at?.remainder }));
  return <div ref={root} className="world-tension-drill">
    <DrillParts virtual active={active} onVisible={onVisible} signedShares
      parts={at?.parts.map(part => {
        const sample = samples.get(part.key);
        return { id: part.key, label: part.preparing ? 'Still being prepared' : visibleSet.has(part.key) ? countryLabel(part.key) : 'Country name unavailable',
          value: contributionPercent(model, part.raw), raw: BigInt(part.raw), text: exact(part.raw),
          shareText: tensionShareText(part.raw, at?.read.sumRaw ?? null), color: colours.get(part.key) ?? tokens.muted,
          icon: sample ? flags.get(nameKey(sample.hour, sample.tag)) : undefined };
      }) ?? []}
      format={value => `${value}%`} total={at ? contributionPercent(model, at.read.sumRaw) : null}
      unattributed={at ? { label: tensionReadPreparing(at.read, 'unnamedRaw') ? 'Contributor attribution still being prepared' : 'Other', value: contributionPercent(model, at.remainder), raw: BigInt(at.remainder),
        text: exact(at.remainder), shareText: tensionShareText(at.remainder, at.read.sumRaw), color: tokens.muted } : undefined}
      totalRow={{ label: 'Total contributions', value: at ? exact(at.read.sumRaw) : 'Not recorded' }} />
    <p className="drill-empty">World tension: {at ? formatTension(model, at.read.tensionRaw, model.tensionDecimals) : 'Not recorded'}.</p>
    {prepared.data?.preparing?.length ? <p className="chart-line-key">Contributor history and events are still being prepared.</p> : null}
    <PreparedKey approximate={prepared.data?.summaries.world?.approximate} pending={prepared.data?.pending} />
    <div className="world-tension-cause">Cause
      <Select label="Cause" value={causeIndex < 0 ? '' : eventKeys[causeIndex]} onChange={value => onCause(value === '' ? null : value)}
        options={[{ value: '', label: 'Choose a cause' }, ...model.reply.events.map((event, index) => ({ value: eventKeys[index], label: `${dayLabel(event.hour)} · ${event.reason}` }))]} />
    </div>
    {cause && <div className="drill-empty" role="group" aria-label="Selected cause">
      <strong>{cause.reason}</strong><p>{formatDateTime(cause.hour)} · {cause.tag ?? 'Country not identified'}.</p>
      <p>Final contribution: {causeRead ? exact(causeRead.addedRaw) : 'Not yet recorded'}.</p>
    </div>}
    <div className="world-tension-drill-chart"><TimeChart label="Country contributions over time" extent={extent} playhead={playhead}
      playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
      hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={[pendingLayer(prepared.data?.pending), ...layers]} active={active}
      formatValue={formatTensionAxis} keyboardHour={prepared.keyboardHour} keyboardRevision={prepared.keyboardRevision} onNavigateHour={prepared.onNavigateHour}
      onHover={setHoverHour}
      onSummaryView={view => prepared.onSummaryView(view,keys.map(key => {
        const country = prepared.data?.catalogue.find(country => countryKey(country) === key);
        return { id: JSON.stringify(key),spans: country ? [{ tag: country.key,firstHour: track.branch.firstHour,lastHour: track.branch.lastHour,startsSegment: false }] : [] };
      }))}
      revision={{ hour, styles, flags, labels: tooltipLabels.labelAt, viewport }} pickOwner={pickOwner} onPick={onPick}
      tooltip={shown => {
        const value = tensionDrillAt(model, shown);
        if (!value && !prepared.data?.exactHours.includes(Math.floor(shown))) return { title: formatDateTime(shown),rows: [{ label: 'Country contributions',value: 'Loading…' }] };
        if (!value) return { title: formatDateTime(shown), rows: [{ label: 'Country contributions', value: 'Not recorded' }] };
        const tipSamples = new Map(value.parts.flatMap(part => {
          const sample = tensionLookAt(model, part.key, shown, track.branch);
          return sample ? [[part.key, sample] as const] : [];
        }));
        const countryRows = value.parts.map(part => {
          const sample = tipSamples.get(part.key);
          const label = part.preparing ? { name: 'Still being prepared', flag: undefined } : sample ? tooltipLabels.labelAt(sample.hour)(sample.tag) : { name: 'Country name unavailable', flag: undefined };
          return { label: label.name, countryFlag: sample ? label : undefined, swatch: colours.get(part.key) ?? tokens.muted, value: exact(part.raw) };
        });
        const rowKeys = new Map<TooltipRow, string>(countryRows.map((row, index) => [row, value.parts[index].key]));
        const fitted = fitTooltip([{ title: formatDateTime(shown), rows: [
          ...countryRows,
          ...(value.remainder !== '0' ? [{ label: 'Other', value: exact(value.remainder), swatch: tokens.muted }] : []),
          { label: 'Total contributions', value: exact(value.read.sumRaw), strong: true, divider: true },
          { label: 'World tension', value: formatTension(model, value.read.tensionRaw, model.tensionDecimals) },
        ] }], viewport, { keep: 2 });
        ask(fitted.rows.flatMap(row => {
          const key = rowKeys.get(row);
          const sample = key == null ? null : tipSamples.get(key);
          return sample ? [sample] : [];
        }));
        return fitted;
      }} onContextMenu={(shown, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, shown)])} /></div>
    {menu.menu}
  </div>;
}
