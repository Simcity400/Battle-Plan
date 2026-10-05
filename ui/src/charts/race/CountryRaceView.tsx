import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
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
import { extentOfAll, lineLabelHours, runAt } from '../series';
import type { Run } from '../series';
import { buildStack, stackedSegments, stackExtent } from '../stack';
import TimeChart from '../TimeChart';
import type { ChartLayer } from '../TimeChart';
import { fitTooltip } from '../tooltip';
import type { TooltipContent } from '../tooltip';
import { useChartTokens, useViewportHeight } from '../useChartTokens';
import { shownUntil } from '../view';
import type { PlaybackMode } from '../view';
import { useCountryColours } from './countryColours';
import { countryRaceItems, heldStackSegment, partColours } from './countryRace';
import type { CountryRaceEntry, CountryRaceLine, CountryRacePart } from './countryRace';
import { linesAt, linesTooltip } from './lines';
import type { LabelAt } from './lines';
import type { RaceView } from './options';
import RaceChart from './RaceChart';
import './countryRace.css';

export interface CountryRaceViewProps {
  runtime: WidgetRuntime; track: BoundTrack; active: boolean;
  options: Readonly<Record<string, OptionValue>>; view: RaceView;
  extent: [number, number]; playhead: number | null; moment: number; slots: number;
  ranked: readonly CountryRaceEntry[]; lines: readonly CountryRaceLine[];
  parts?: readonly CountryRacePart[];
  mix?: { tag: string; bands: readonly (readonly Run[])[] } | null;
  segmentAt: (tag: string, hour: number) => number | null;
  tooltip: (tag: string, hour: number, name: string, colours: ReadonlyMap<string, string>) => Iterable<TooltipContent> | null;
  lineTooltip?: (hour: number, tags: readonly string[], labelAt: LabelAt, country: (tag: string) => string,
    colours: ReadonlyMap<string, string>) => Iterable<TooltipContent>;
  label: string; empty: string; formatValue: (value: number) => string;
}
const NO_PARTS: readonly CountryRacePart[] = [];

/** Race, country lines and one-country mix use the same colours, hour-specific
 * identities, exact reads, chart controls and Jump to across all race widgets. */
export default function CountryRaceView({ runtime, track, active, options, view, extent, playhead, moment, slots, ranked, lines,
  parts = NO_PARTS, mix, segmentAt, tooltip, lineTooltip, label, empty, formatValue }: CountryRaceViewProps) {
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const menu = useContextMenu(`${label} actions`);
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
  const tags = useMemo(() => view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lines.map(line => line.tag) : mix ? [mix.tag] : [],
    [view, ranked, lines, mix]);
  const looks = useCountryLooks(runtime.client, track, tags, moment, active, segmentAt, view === 'lines' ? hours : undefined);
  const country = useCountryColours(tags, looks.color, tokens, looks);
  const colours = useMemo(() => partColours(parts, tokens.categories), [parts, tokens.categories]);
  const items = useMemo(() => countryRaceItems(ranked, colours, country, tokens.muted), [ranked, colours, country, tokens.muted]);
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
  const keyHours = useMemo(() => [...new Set(lines.flatMap(line => line.runs.map(run => run.firstHour)))].sort((a, b) => a - b), [lines]);
  const lineLayers = useMemo<ChartLayer[]>(() => [{
    draw: (ctx, frame, chartTokens, state) => {
      const labelHours = lineLabelHours(lines, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), null);
      for (const line of lines) drawLine(ctx, frame, chartTokens, line.runs, country(line.tag, labelHours.get(line.tag)),
        { held: true, until: state.playhead ?? undefined });
    },
    extent: x => extentOfAll(lines.map(line => line.runs), x[0], Math.min(x[1], until ?? x[1])),
  }], [lines, country, until]);
  const stack = useMemo(() => mix ? buildStack(mix.bands) : null, [mix]);
  const mixLayers = useMemo<ChartLayer[]>(() => stack ? [{
    draw: (ctx, frame, chartTokens, state) => {
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of stackedSegments(stack, frame.x[0], Math.min(frame.x[1], state.playhead ?? frame.x[1]), hoursPerPixel / 2)) {
        const held = heldStackSegment(segment);
        drawBands(ctx, frame, chartTokens, held.hours, held.bands, parts.map(part => colours.get(part.id) ?? chartTokens.muted));
      }
    },
    extent: x => stackExtent(stack, x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [stack, parts, colours, until]);
  const revision = useMemo(() => ({ lines, mix, looks: looks.revision, labels: labels.revision, viewport }),
    [lines, mix, looks.revision, labels.revision, viewport]);
  const jump = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  const tipAt = (tag: string, hour: number, name: string) => {
    const forms = tooltip(tag, hour, name, colours);
    return forms ? fitTooltip(forms, viewport, { keep: 1 }) : null;
  };
  const common = { extent, playhead, playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: runtime.clock.getSnapshot().hoursPerSecond, branchStart: track.branch.firstHour, active, onContextMenu: jump,
    axes: { xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: view === 'lines' && options.log === true } };
  if (view === 'race' && !ranked.length) return <WidgetState kind="empty" message={empty} />;
  return <div className="country-race">
    {view === 'mix' && mix && <p className="country-race-country">{looks.name(mix.tag)}</p>}
    {parts.length > 0 && (view === 'race' || view === 'mix') && <Legend label="Types"
      items={parts.map(part => ({ id: part.id, label: part.label, color: colours.get(part.id) ?? tokens.muted }))} />}
    {view === 'lines' && !endLabels && <Legend label="Countries" items={lines.map(line => ({ id: line.tag, label: looks.name(line.tag), color: country(line.tag) }))} />}
    <div className="country-race-chart" onPointerLeave={() => showTip(null)}>
      {view === 'race' ? <RaceChart label={`${label}, ${dayLabel(moment)}`} items={items} slots={slots} name={looks.name} flag={looks.flag}
        tooltip={tag => { showTip(moment, [tag]); return tipAt(tag, moment, labels.labelAt(moment)(tag).name); }}
        onContextMenu={(_, x, y) => jump(moment, x, y)} dateOverlay={options.dateOverlay === true ? dayLabel(moment) : null}
        empty={empty} hoursPerSecond={common.hoursPerSecond} active={active} resetKey={`${track.source.id}:${track.branch.id}`} />
        : <TimeChart {...common} label={`${label} over time`} layers={view === 'lines' ? lineLayers : mixLayers}
          keyHours={view === 'lines' ? keyHours : stack?.hours ?? []} onViewChange={onViewChange}
          padding={view === 'lines' && endLabels ? { right: tokens.fontSize.body * 12 } : undefined}
          revision={revision}
          tooltip={hour => {
            if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
            showTip(hour, view === 'lines' ? linesAt(hour, lines) : mix ? [mix.tag] : []);
            return view === 'lines' ? fitTooltip(lineTooltip?.(hour, linesAt(hour, lines), labels.labelAt, country, colours)
              ?? [linesTooltip(hour, lines, labels.labelAt, country, formatDateTime(hour), label, formatValue)], viewport)
              : mix ? tipAt(mix.tag, hour, labels.labelAt(hour)(mix.tag).name) : null;
          }} overlay={(ctx, frame, chartTokens) => {
            if (view !== 'lines' || !endLabels) return;
            const labelHours = lineLabelHours(lines, frame.x[0], Math.min(frame.x[1], extent[1]), until);
            placeEndLabels(ctx, frame, chartTokens, lines.flatMap(line => {
              const hour = labelHours.get(line.tag);
              const run = hour == null ? null : runAt(line.runs, hour);
              return run ? [{ id: line.tag, color: country(line.tag, hour), y: frame.toY(run.value), text: `${looks.name(line.tag, hour)} ${formatValue(run.value)}` }] : [];
            }));
          }} />}
    </div>
    {menu.menu}
  </div>;
}
