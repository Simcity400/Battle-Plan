import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import type { IconName } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionValue } from '../../dashboard/options';
import { dayLabel, drawBands, drawLine, placeEndLabels } from '../draw';
import { jumpItem } from '../jump';
import Legend from '../Legend';
import { extentOfAll, lineLabelHours } from '../series';
import type { Run } from '../series';
import { buildStack, stackExtent } from '../stack';
import TimeChart from '../TimeChart';
import type { ChartLayer } from '../TimeChart';
import type { ChartStrip } from '../TimeChart';
import { CHART_LAYOUT } from '../tokens';
import { fitTooltip } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { shownUntil } from '../view';
import type { PlaybackMode } from '../view';
import { useCountryColours } from './countryColours';
import { countryRaceEnds, countryRaceHours, countryRaceItems, countryRaceSegments, partColours } from './countryRace';
import type { CountryRaceEntry, CountryRaceLine, CountryRacePart } from './countryRace';
import { linesAt, linesTooltip } from './lines';
import type { LabelAt } from './lines';
import type { RaceView } from './options';
import RaceChart from './RaceChart';
import './countryRace.css';

export interface CountryRaceViewProps {
  runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean;
  options: Readonly<Record<string, OptionValue>>; view: RaceView;
  extent: [number, number]; playhead: number | null; moment: number; slots: number;
  ranked: readonly CountryRaceEntry[]; lines: readonly CountryRaceLine[];
  parts?: readonly CountryRacePart[];
  raceLegendParts?: readonly CountryRacePart[]; partsLabel?: string;
  mix?: { tag: string; bands: readonly (readonly Run[])[]; parts?: readonly CountryRacePart[] } | null;
  countryTags?: readonly string[];
  segmentAt?: (tag: string, hour: number) => number | null;
  tooltip: (tag: string, hour: number, name: string, colours: ReadonlyMap<string, string>) => Iterable<TooltipContent> | null;
  lineTooltip?: (hour: number, tags: readonly string[], labelAt: LabelAt, country: (tag: string) => string,
    colours: ReadonlyMap<string, string>) => Iterable<TooltipContent>;
  timeTooltip?: (hour: number) => Iterable<TooltipContent> | null;
  keyHours?: readonly number[]; strip?: ChartStrip; beforeChart?: ReactNode;
  /** Keeps country requests mounted while the widget shows its product state. */
  state?: ReactNode;
  labels?: { race?: string; lines?: string; mix?: (name: string) => string; actions?: string };
  lineEmpty?: { icon?: IconName; message: string };
  mixEmpty?: { icon?: IconName; message: string; unavailable: (name: string) => string };
  resetKey?: string; tooltipResetKey?: unknown;
  presentation?: {
    exact?: boolean; held?: boolean; mixSamplePixels?: number;
    lineLooks?: 'end' | 'moment'; lineColourHours?: 'drawn' | 'labels' | 'moment'; countryLooks?: boolean;
    raceTooltip?: 'hover' | 'sampled'; tooltipKeep?: number; linePadding?: number;
    strongMixName?: boolean; mixKeyHours?: 'boundaries' | 'starts'; raceEmptyState?: boolean;
    resetTimeView?: boolean;
  };
  label: string; empty: string; formatValue: (value: number) => string;
}
const NO_PARTS: readonly CountryRacePart[] = [];
const NO_TAGS: readonly string[] = [];

/** Race, country lines and one-country mix use the same colours, hour-specific
 * identities, exact reads, chart controls and Jump to across all race widgets. */
export default function CountryRaceView({ runtime, track, active, options, view, extent, playhead, moment, slots, ranked, lines,
  parts = NO_PARTS, raceLegendParts = parts, partsLabel = 'Types', mix, countryTags, segmentAt, tooltip, lineTooltip, timeTooltip,
  keyHours: suppliedHours, strip, beforeChart, state, labels: chartLabels, lineEmpty, mixEmpty, resetKey, tooltipResetKey,
  presentation = {}, label, empty, formatValue }: CountryRaceViewProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const menu = useContextMenu(chartLabels?.actions ?? `${label} actions`);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const endLabels = options.endLabels !== false;
  const [labelView, setLabelView] = useState<[number, number] | null>(null);
  const onViewChange = useCallback((x: readonly [number, number]) => {
    const bounds: [number, number] = [Math.floor(x[0]), Math.floor(x[1])];
    setLabelView(before => before?.[0] === bounds[0] && before[1] === bounds[1] ? before : bounds);
  }, []);
  const hours = useMemo(() => lineLabelHours(lines, endLabels ? Math.max(extent[0], labelView?.[0] ?? extent[0]) : extent[0],
    endLabels ? Math.min(extent[1], labelView?.[1] ?? extent[1]) : extent[1], until), [lines, endLabels, extent, labelView, until]);
  const tags = useMemo(() => presentation.countryLooks === false ? NO_TAGS : countryTags
    ?? (view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lines.map(line => line.tag) : mix ? [mix.tag] : []),
    [presentation.countryLooks, countryTags, view, ranked, lines, mix]);
  const looks = useCountryLooks(runtime.client, track, tags, moment, active, segmentAt,
    view === 'lines' && presentation.lineLooks !== 'moment' ? hours : undefined);
  const country = useCountryColours(tags, looks.color, tokens, looks);
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const items = useMemo(() => countryRaceItems(ranked, colours, country, tokens.muted, presentation.exact !== false),
    [ranked, colours, country, tokens.muted, presentation.exact]);
  const [tip, setTip] = useState<{ hour: number; tags: readonly string[] } | null>(null);
  const tipKey = useRef('');
  const showTip = (hour: number | null, wanted: readonly string[] = []) => {
    const key = hour == null ? '' : JSON.stringify([hour, wanted]);
    if (key === tipKey.current) return;
    tipKey.current = key;
    setTip(hour == null ? null : { hour, tags: wanted });
  };
  const wanted = useMemo(() => tip ? tip.tags.map(tag => ({ hour: tip.hour, tag })) : [], [tip]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  useEffect(() => { if (!active) { tipKey.current = ''; setTip(null); } }, [active]);
  useEffect(() => { if (tooltipResetKey !== undefined) { tipKey.current = ''; setTip(null); } }, [tooltipResetKey]);
  const keyHours = useMemo(() => countryRaceHours(lines), [lines]);
  const lineLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const labelHours = presentation.lineColourHours === 'moment' ? undefined : presentation.lineColourHours === 'labels'
        ? endLabels ? lineLabelHours(lines, frame.x[0], Math.min(frame.x[1], extent[1]), until) : undefined
        : lineLabelHours(lines, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), null);
      for (const line of lines) drawLine(ctx, frame, chartTokens, line.runs, line.color ?? country(line.tag, labelHours?.get(line.tag)),
        { held: presentation.held !== false, until: state.playhead ?? undefined, area: line.area, dash: line.dash, endDot: line.endDot });
    },
    extent: x => extentOfAll(lines.map(line => line.runs), x[0], Math.min(x[1], until ?? x[1])),
  }], [lines, country, until, extent, endLabels, presentation.held, presentation.lineColourHours]);
  const mixBands = mix?.bands;
  const stack = useMemo(() => mixBands ? buildStack(mixBands) : null, [mixBands]);
  const mixParts = mix?.parts ?? parts;
  const mixColours = useMemo(() => partColours(mixParts, tokens.categories, colours), [mixParts, colours, tokens.categories]);
  const mixHours = useMemo(() => mixBands ? countryRaceHours(mixBands.map((runs, index) => ({ tag: String(index), runs }))) : [], [mixBands]);
  const mixLayers = useMemo<ChartLayer[]>(() => stack ? [{
    draw: (ctx, frame, chartTokens, state) => {
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of countryRaceSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]),
        hoursPerPixel, presentation.mixSamplePixels, presentation.held !== false)) {
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, mixParts.map(part => mixColours.get(part.id) ?? chartTokens.muted));
      }
    },
    extent: x => stackExtent(stack, x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [stack, mixParts, mixColours, until, presentation.mixSamplePixels, presentation.held]);
  const revision = useMemo(() => ({ lines, mix, looks: looks.revision, labels: labels.revision, viewport }),
    [lines, mix, looks.revision, labels.revision, viewport]);
  if (state != null || !track) return state ?? null;
  const jump = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  const tipAt = (tag: string, hour: number, name: string) => {
    const forms = tooltip(tag, hour, name, colours);
    return forms ? fitTooltip(forms, viewport, { keep: presentation.tooltipKeep ?? 1 }) : null;
  };
  const common = { extent, playhead, playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: runtime.clock.getSnapshot().hoursPerSecond, branchStart: track.branch.firstHour, active, onContextMenu: jump,
    axes: { xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: view === 'lines' && options.log === true } };
  if (view === 'race' && !ranked.length && presentation.raceEmptyState !== false) return <WidgetState kind="empty" message={empty} />;
  if (view === 'lines' && !lines.length && lineEmpty) return <WidgetState kind="empty" {...lineEmpty} />;
  if (view === 'mix' && mixEmpty) {
    if (!mix) return <WidgetState kind="empty" icon={mixEmpty.icon} message={mixEmpty.message} />;
    if (!mixHours.length) return <WidgetState kind="unavailable" message={mixEmpty.unavailable(looks.name(mix.tag))} />;
  }
  const legend = (shown: readonly CountryRacePart[], map: ReadonlyMap<string, string>) => <Legend label={partsLabel}
    items={shown.map(part => ({ id: part.id, label: part.label, color: part.color ?? map.get(part.id) ?? tokens.muted, value: part.value }))} />;
  return <div className="country-race">
    {view === 'mix' && mix && <p className={`country-race-country${presentation.strongMixName ? ' is-strong' : ''}`}>{looks.name(mix.tag)}</p>}
    {view === 'race' && legend(raceLegendParts, colours)}
    {view === 'mix' && legend(mixParts, mixColours)}
    {view === 'lines' && !endLabels && presentation.countryLooks !== false && <Legend label="Countries"
      items={lines.map(line => ({ id: line.tag, label: looks.name(line.tag), color: country(line.tag) }))} />}
    {beforeChart}
    <div className="country-race-chart" onPointerLeave={() => showTip(null)}>
      {view === 'race' ? <RaceChart label={chartLabels?.race ?? `${label}, ${dayLabel(moment)}`} items={items} slots={slots} name={looks.name} flag={looks.flag}
        tooltip={tag => {
          if (presentation.raceTooltip === 'sampled') return tipAt(tag, moment, looks.name(tag));
          showTip(moment, [tag]); return tipAt(tag, moment, labels.labelAt(moment)(tag).name);
        }}
        onContextMenu={(_, x, y) => jump(moment, x, y)} dateOverlay={options.dateOverlay === true ? dayLabel(moment) : null}
        empty={empty} hoursPerSecond={common.hoursPerSecond} active={active} resetKey={resetKey ?? `${track.source.id}:${track.branch.id}`} />
        : <TimeChart key={presentation.resetTimeView ? view : undefined} {...common} label={view === 'mix' && mix ? chartLabels?.mix?.(looks.name(mix.tag)) ?? `${label} over time`
          : chartLabels?.lines ?? `${label} over time`} layers={view === 'lines' ? lineLayers : mixLayers} strip={strip}
          keyHours={suppliedHours ?? (view === 'lines' ? keyHours : presentation.mixKeyHours === 'starts' ? mixHours : stack?.hours ?? [])}
          onViewChange={presentation.lineLooks === 'moment' || (view === 'mix' && presentation.mixKeyHours === 'starts') ? undefined : onViewChange}
          padding={view === 'lines' ? presentation.linePadding !== undefined
            ? { right: endLabels ? presentation.linePadding : CHART_LAYOUT.right } : endLabels ? { right: tokens.fontSize.body * 12 } : undefined : undefined}
          revision={revision}
          tooltip={hour => {
            if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
            if (timeTooltip) {
              const forms = timeTooltip(hour);
              return forms ? fitTooltip(forms, viewport, { keep: presentation.tooltipKeep ?? 1 }) : null;
            }
            showTip(hour, view === 'lines' ? linesAt(hour, lines) : mix ? [mix.tag] : []);
            return view === 'lines' ? fitTooltip(lineTooltip?.(hour, linesAt(hour, lines), labels.labelAt, country, colours)
              ?? [linesTooltip(hour, lines, labels.labelAt, country, formatDateTime(hour), label, formatValue)], viewport)
              : mix ? tipAt(mix.tag, hour, labels.labelAt(hour)(mix.tag).name) : null;
          }} overlay={(ctx, frame, chartTokens) => {
            if (view !== 'lines' || !endLabels) return;
            placeEndLabels(ctx, frame, chartTokens, countryRaceEnds(lines, frame.x[0], Math.min(frame.x[1], extent[1]), until,
              looks.name, country, formatValue, presentation.lineLooks !== 'moment').map(end => ({ ...end, y: frame.toY(end.value) })));
          }} />}
    </div>
    {menu.menu}
  </div>;
}
