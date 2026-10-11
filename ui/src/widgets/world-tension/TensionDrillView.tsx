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
import TimeChart from '../../charts/TimeChart';
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
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { parseColor, toHex } from '../../shell/appearance/color';
import { contributionPercent, formatContribution, formatTension, formatTensionAxis } from './tensionData';
import type { TensionModel } from './tensionData';
import { tensionCauseAt, tensionDrillAt, tensionDrillHistory, tensionLookAt, tensionShareText, tensionVisibleParts } from './tensionDrill';

/** The full native country stack. Event final amounts stay separate because the
 * product does not record source current amounts or per-read source presence. */
export default function TensionDrill({ model, runtime, track, active, options, extent, playhead, hour, selectedCause, onCause, onPick }: {
  model: TensionModel; runtime: WidgetRuntime; track: BoundTrack; active: boolean;
  options: Readonly<Record<string, OptionValue>>; extent: [number, number]; playhead: number | null; hour: number;
  selectedCause: number | null; onCause: (index: number | null) => void; onPick: (hour: number) => void;
}) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
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
  const flags = useCountryFlagsAt(runtime.client, track, visibleWanted, active, 'shield', maxHours);
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
  const keyHours = useMemo(() => model.reply.reads.map(read => read.firstHour), [model.reply.reads]);
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
  const pickOwner = useMemo(() => ({ source: [track.source.id, track.source.revision, track.branch.id], model: model.reply }),
    [track.source.id, track.source.revision, track.branch.id, model.reply]);
  const playback = options.playback as PlaybackMode;
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const layers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      forEachCompactStackSegment(stack, frame.x[0], to, (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth) / 2, piece => {
        const segment = heldStackSegment(piece);
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, layerColours, segment);
      });
      drawLine(ctx, frame, chartTokens, history.world, chartTokens.text, { until: to, held: true });
      drawPlayhead(ctx, frame, chartTokens, hour);
    },
    extent: x => {
      const to = Math.min(x[1], until ?? x[1]);
      const bands = compactStackExtent(stack, x[0], to);
      const world = extentOf(history.world, x[0], to);
      return bands && world ? [Math.min(bands[0], world[0]), Math.max(bands[1], world[1])] : bands ?? world;
    },
  }], [stack, history, layerColours, hour, until]);
  const menu = useContextMenu('Country contribution actions');
  const cause = selectedCause == null ? null : model.reply.events[selectedCause] ?? null;
  const causeRead = cause ? tensionCauseAt(cause, hour) : null;
  const exact = (raw: string) => formatContribution(model, raw, true, model.contributionDecimals);
  return <div className="world-tension-drill">
    <DrillParts virtual active={active} onVisible={onVisible} signedShares
      parts={at?.parts.map(part => {
        const sample = samples.get(part.key);
        return { id: part.key, label: visibleSet.has(part.key) ? countryLabel(part.key) : 'Country name unavailable',
          value: contributionPercent(model, part.raw), raw: BigInt(part.raw), text: exact(part.raw),
          shareText: tensionShareText(part.raw, at?.read.sumRaw ?? null), color: colours.get(part.key) ?? tokens.muted,
          icon: sample ? flags.get(nameKey(sample.hour, sample.tag)) : undefined };
      }) ?? []}
      format={value => `${value}%`} total={at ? contributionPercent(model, at.read.sumRaw) : null}
      unattributed={at ? { label: 'Other', value: contributionPercent(model, at.remainder), raw: BigInt(at.remainder),
        text: exact(at.remainder), shareText: tensionShareText(at.remainder, at.read.sumRaw), color: tokens.muted } : undefined}
      totalRow={{ label: 'Total contributions', value: at ? exact(at.read.sumRaw) : 'Not recorded' }} />
    <p className="drill-empty">World tension: {at ? formatTension(model, at.read.tensionRaw, model.tensionDecimals) : 'Not recorded'}.</p>
    <div className="world-tension-cause">Cause
      <Select label="Cause" value={selectedCause == null ? '' : String(selectedCause)} onChange={value => onCause(value === '' ? null : Number(value))}
        options={[{ value: '', label: 'Choose a cause' }, ...model.reply.events.map((event, index) => ({ value: String(index), label: `${dayLabel(event.hour)} · ${event.reason}` }))]} />
    </div>
    {cause && <div className="drill-empty" role="group" aria-label="Selected cause">
      <strong>{cause.reason}</strong><p>{formatDateTime(cause.hour)} · {cause.tag ?? 'Country not identified'}.</p>
      <p>Final contribution: {causeRead ? exact(causeRead.addedRaw) : 'Not yet recorded'}.</p>
    </div>}
    <div className="world-tension-drill-chart"><TimeChart label="Country contributions over time" extent={extent} playhead={playhead}
      playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
      hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
      axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }} layers={layers} active={active}
      formatValue={formatTensionAxis} keyHours={keyHours}
      revision={{ hour, styles, flags, labels: tooltipLabels.labelAt, viewport }} pickOwner={pickOwner} onPick={onPick}
      tooltip={shown => {
        const value = tensionDrillAt(model, shown);
        if (!value) return { title: formatDateTime(shown), rows: [{ label: 'Country contributions', value: 'Not recorded' }] };
        const tipSamples = new Map(value.parts.flatMap(part => {
          const sample = tensionLookAt(model, part.key, shown, track.branch);
          return sample ? [[part.key, sample] as const] : [];
        }));
        const countryRows = value.parts.map(part => {
          const sample = tipSamples.get(part.key);
          const label = sample ? tooltipLabels.labelAt(sample.hour)(sample.tag) : { name: 'Country name unavailable', flag: undefined };
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
