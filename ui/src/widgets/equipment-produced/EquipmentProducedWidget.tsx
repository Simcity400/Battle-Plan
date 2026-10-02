import { useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, drawBands, drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { useCountryColours } from '../../charts/race/countryColours';
import { linesAt, linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { RaceView } from '../../charts/race/options';
import type { RaceItem } from '../../charts/race/race';
import RaceChart from '../../charts/race/RaceChart';
import { extentOfAll, lineEndHour, runAt } from '../../charts/series';
import type { Span } from '../../charts/series';
import { buildStack, stackedSegments, stackExtent } from '../../charts/stack';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { NameAt } from '../../dashboard/countryFacts';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { EquipmentProducedReply } from '../../generated/dashboard';
import { MAX_COUNTRIES, OPTIONS } from './definition';
import { bandLabel, bandValue, changeHours, emptyRace, foldsPerCountry, formatUnits, keyBands, MEASURES, mixBands, producedAt, producedModel, producedRequest, producedTooltips,
  raceEntries, rankProduced, seriesBands, totalRuns, validateProducedReply } from './producedData';
import type { Grouping, KeyBand, Measure } from './producedData';
import './equipmentProduced.css';

const NO_WARNINGS: string[] = [];
/** Room for the names at the lines' ends. */
const END_LABEL_ROOM = 150;

/** Equipment each country produced (spec 4.2 Equipment produced): a race re-ranked
 * every hour, the same values as lines over time, or one country's equipment stacked
 * by group. */
export default function EquipmentProducedWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (options.view as RaceView) ?? 'race';
  const measure = (options.measure as Measure) ?? 'total';
  const grouping = (options.grouping as Grouping) ?? 'type';
  const included = useMemo(() => contentSet(options.included).map(contentKey), [options.included]);
  const refits = options.refits === true;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  // The moment a race or a mix shows: the playhead, the end of a range, or the branch end.
  const moment = Math.floor(playhead ?? extent[1]);
  const lastHour = track?.branch.lastHour ?? 0;
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const groupReply = useCountryGroups(runtime, track, active && grouped);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groupReply.data, lastHour, MAX_COUNTRIES),
    [widget.countries, fallback, groupReply.data, lastHour]);
  // Only what the widget draws is asked for: the chosen countries, or the top N, which
  // the product ranks at every hour and cuts to the hours each was among them.
  const params = track ? producedRequest(track.branch.id, resolved, measure, grouping, included, refits) : null;
  const request = track && params && (params.top || params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'equipment-produced', params } : null;
  const { data, error, retry } = useProduct<EquipmentProducedReply>(runtime.client, request, active, validateProducedReply);
  const model = useMemo(() => data ? producedModel(data) : null, [data]);
  const bands = useMemo(() => model ? keyBands(model, tokens.categories, tokens.muted) : [], [model, tokens]);
  // A country's bands: by exact variant each keeps its own largest in colour (user
  // decision, 29 Sep), else the reply's, the same for every country.
  const bandsFor = useMemo(() => {
    const own = new Map<string, KeyBand[]>();
    return (tag: string): readonly KeyBand[] => {
      if (!model || !foldsPerCountry(model)) return bands;
      if (!own.has(tag)) own.set(tag, seriesBands(model, tag, tokens.categories, tokens.muted));
      return own.get(tag)!;
    };
  }, [model, bands, tokens]);
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  // The top N rank only countries that have produced something (user decision).
  const produced = resolved.top != null;
  // The hours a country is shown: in the top N (from the product), or a member of a
  // group counted at each hour; null shows it whenever recorded.
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;

  // The countries at the moment, ranked by the value shown.
  const candidates = useMemo(() => model ? rankedCandidates(resolved, moment, model.tags) : [], [model, resolved, moment]);
  const ranked = useMemo(() => model ? rankProduced(model, candidates, moment, slots, produced) : [],
    [model, candidates, moment, slots, produced]);
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  const looks = useCountryLooks(runtime.client, track, shownTags, moment, active);
  const colorOf = useCountryColours(shownTags, looks.color, tokens, looks);

  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groupReply.data?.warnings ?? []),
      ...(resolved.dropped ? [`${resolved.dropped} more countries are left out; this widget shows at most ${MAX_COUNTRIES}.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groupReply.data, resolved.dropped, onDiagnostics]);

  const split = options.split === true;
  const items = useMemo<RaceItem[]>(() => model ? raceEntries(model, ranked, { split, country: colorOf, bands: bandsFor, unknown: tokens.muted }) : [],
    [model, ranked, split, colorOf, bandsFor, tokens.muted]);

  const menu = useContextMenu('Equipment produced actions');
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);

  // The hours still to cut a country's runs to: a group's, counted at each hour. A
  // top-N reply's samples are already cut to its hours, so they are not cut again.
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;
  // The hours shown: a date range, or the branch. Nothing outside it is drawn, fitted
  // or hovered, even where an axis is dragged past it.
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // Lines over time: one per country, cut to the hours it is shown.
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const runs = totalRuns(model, tag, cutTo(tag), range);
    return runs.length ? [{ tag, runs }] : [];
  }), [model, view, lineTags, resolved.membership, range]); // eslint-disable-line react-hooks/exhaustive-deps
  // A tooltip names countries as recorded at the hour it is about (`useCountryLabelsAt`):
  // only the countries of the tooltip shown are asked for.
  const [tip, setTip] = useState<{ hour: number; tags: readonly string[] } | null>(null);
  const tipKey = useRef('');
  const showTip = (hour: number | null, tags: readonly string[] = []) => {
    const key = hour == null ? '' : JSON.stringify([hour, tags]);
    if (key === tipKey.current) return;
    tipKey.current = key;
    setTip(hour == null ? null : { hour, tags });
  };
  const tipWanted = useMemo<NameAt[]>(() => tip ? tip.tags.map(tag => ({ hour: tip.hour, tag })) : [], [tip]);
  const labels = useCountryLabelsAt(runtime.client, track, tipWanted, active);
  useEffect(() => { if (!active) showTip(null); }, [active]); // eslint-disable-line react-hooks/exhaustive-deps
  const lineHours = useMemo(() => [...new Set(lines.flatMap(line => changeHours(line.runs)))].sort((a, b) => a - b), [lines]);
  const lineLayers = useMemo<ChartLayer[]>(() => lines.length ? [{
    draw: (ctx, frame, chartTokens, state) => {
      for (const line of lines) drawLine(ctx, frame, chartTokens, line.runs, colorOf(line.tag), { until: state.playhead ?? undefined });
    },
    extent: x => extentOfAll(lines.map(line => line.runs), x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [lines, colorOf, until]);

  // One country's mix: its groups stacked, cut to the hours it is shown.
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const mixCut = mixTag ? cutTo(mixTag) : null;
  // The mix's bands: its country's own (by variant) or the reply's.
  const mixKeyBands = useMemo(() => mixTag ? bandsFor(mixTag) : bands, [mixTag, bandsFor, bands]);
  const mix = useMemo(() => {
    if (!model || view !== 'mix' || !mixTag) return null;
    const runs = mixBands(model, mixTag, mixKeyBands, mixCut, range);
    const hours = [...new Set(runs.flatMap(band => band.map(run => run.firstHour)))].sort((a, b) => a - b);
    return { stack: buildStack(runs), hours };
  }, [model, view, mixTag, mixKeyBands, mixCut, range]);
  const mixLayers = useMemo<ChartLayer[]>(() => mix ? [{
    draw: (ctx, frame, chartTokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of stackedSegments(mix.stack, frame.x[0], to, hoursPerPixel / 2)) {
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, mixKeyBands.map(band => band.color));
      }
    },
    extent: x => stackExtent(mix.stack, x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [mix, mixKeyBands, until]);

  if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see the equipment it produced." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (grouped && groupReply.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" icon="layers" message={resolved.reason} />;
  // A group's members are not known until its history arrives; nothing is shown as empty meanwhile.
  if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading equipment produced…" />;
  if (!data.reads.length) return <WidgetState kind="unavailable" message="Not in this recording: it has no count of the equipment production lines delivered." />;
  const measureInfo = MEASURES.find(item => item.value === measure) ?? MEASURES[0];
  // Per week or month, a value needs a read of the same country that long before: a
  // recording shorter than that has none, which is not the same as no production.
  if (!produced && !model.tags.length) return <WidgetState kind="unavailable" message={measure === 'total' ? 'No production was recorded for these countries in this branch.'
    : `No ${measureInfo.label.toLowerCase()} value yet: each needs a read of the same country a ${measure === 'week' ? 'week' : 'month'} before, and this branch has none that long.`} />;
  // `name` names the country as recorded at the hour the tooltip is about.
  const tooltipAt = (tag: string, hour: number, name: string) => {
    const at = isMember(membershipOf(tag) ?? undefined, hour) ? producedAt(model, tag, hour) : null;
    return at ? fitTooltip(producedTooltips(model, tag, name, at, hour, dayLabel(hour), bandsFor(tag)), viewport, { keep: 1 }) : null;
  };
  const axes = { xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: view === 'lines' && options.log === true };
  const jump = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);
  const legend = (shown: readonly KeyBand[], hour: number, values?: (index: number) => string) => <Legend label="Equipment" items={shown.map((band, index) => ({
    id: band.id, label: bandLabel(model, band, hour), color: band.color, ...(values ? { value: values(index) } : {}) }))} />;
  const window_ = measure === 'week' ? 'week' : 'month';
  // The top N show a country once it has produced something; say so when none has.
  const nothingYet = produced && emptyRace(model, moment) === 'nothing'
    ? (measure === 'total' ? `No country has produced anything by ${dayLabel(moment)}; a country appears here once it has.`
      : `No country produced anything in the ${window_} before ${dayLabel(moment)}; a country appears here once it has.`)
    : measure === 'total' ? `No production recorded on ${dayLabel(moment)}.`
      : `Nothing to show on ${dayLabel(moment)}: ${measureInfo.label.toLowerCase()} needs unbroken reads over the ${window_} before.`;

  if (view === 'race') {
    return <div className="equipment-produced">
      {split && (foldsPerCountry(model)
        ? <p className="equipment-produced-note">Each bar keeps its own country’s largest variants in colour; the rest are Other. Hover a bar to name them.</p>
        : bands.length > 1 && legend(bands, moment))}
      <div className="equipment-produced-chart">
        <RaceChart label={`Equipment ${measureInfo.unit}, ${dayLabel(moment)}`} items={items} slots={slots} name={looks.name} flag={looks.flag}
          tooltip={tag => tooltipAt(tag, moment, looks.name(tag))} onContextMenu={(_, x, y) => jump(moment, x, y)}
          dateOverlay={options.dateOverlay === true ? dayLabel(moment) : null}
          empty={nothingYet} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
          active={active} resetKey={`${track.source.id}:${track.branch.id}:${measure}:${grouping}`} />
      </div>
      {menu.menu}
    </div>;
  }

  const common = {
    extent, playhead, playback, afterPlayhead: options.afterPlayhead === true, windowHours: Number(options.windowDays ?? 365) * 24,
    hoursPerSecond: runtime.clock.getSnapshot().hoursPerSecond, branchStart: track.branch.firstHour, axes, active,
    onContextMenu: jump,
  };
  if (view === 'lines') {
    if (!lines.length) return <WidgetState kind="empty" icon="layers" message="No countries to draw. Choose countries in the widget settings." />;
    const endLabels = options.endLabels !== false;
    return <div className="equipment-produced">
      {!endLabels && <Legend label="Countries" items={lines.map(line => ({ id: line.tag, label: looks.name(line.tag), color: colorOf(line.tag) }))} />}
      <div className="equipment-produced-chart" onPointerLeave={() => showTip(null)}>
        <TimeChart {...common} label={`Equipment ${measureInfo.unit} over time`} layers={lineLayers} keyHours={lineHours}
          padding={{ right: endLabels ? END_LABEL_ROOM : 14 }} revision={JSON.stringify([lines.length, looks.revision, labels.revision, viewport])}
          tooltip={hour => {
            if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
            showTip(hour, linesAt(hour, lines));
            return fitTooltip([linesTooltip(hour, lines, labels.labelAt, colorOf, dayLabel(hour), measureInfo.label, formatUnits)], viewport);
          }}
          overlay={(ctx, frame, chartTokens) => {
            if (!endLabels) return;
            const ends = lines.flatMap(line => {
              const last = lineEndHour(line.runs, frame.x[0], Math.min(until ?? extent[1], frame.x[1], extent[1]));
              const run = last == null ? null : runAt(line.runs, last);
              return run ? [{ id: line.tag, color: colorOf(line.tag), y: frame.toY(run.value), text: `${looks.name(line.tag)} ${formatUnits(run.value)}` }] : [];
            });
            placeEndLabels(ctx, frame, chartTokens, ends);
          }} />
      </div>
      {menu.menu}
    </div>;
  }

  if (!mixTag || !mix) return <WidgetState kind="empty" icon="layers"
    message={widget.countries.kind === 'group' ? `No country of this group was recorded on ${dayLabel(moment)}.` : 'Choose a country in the widget settings.'} />;
  if (!mix.hours.length) return <WidgetState kind="unavailable" message={`No production was recorded for ${looks.name(mixTag)} in this branch.`} />;
  const mixAt = isMember(mixMembership ?? undefined, moment) ? producedAt(model, mixTag, moment) : null;
  const mixSeries = model.series.get(mixTag);
  return <div className="equipment-produced">
    <p className="equipment-produced-country">{looks.name(mixTag)}</p>
    {legend(mixKeyBands, moment, index => {
      const value = mixAt && mixSeries ? bandValue(mixSeries, mixKeyBands[index], mixAt.index) : Number.NaN;
      return Number.isNaN(value) ? '—' : formatUnits(value);
    })}
    <div className="equipment-produced-chart" onPointerLeave={() => showTip(null)}>
      <TimeChart {...common} label={`Equipment of ${looks.name(mixTag)} ${measureInfo.unit}, by ${grouping}`} layers={mixLayers} keyHours={mix.hours}
        revision={JSON.stringify([mixTag, mix.hours.length, viewport, bands.map(band => band.id), labels.revision])}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
          showTip(hour, [mixTag]);
          return tooltipAt(mixTag, hour, labels.labelAt(hour)(mixTag).name);
        }} />
    </div>
    {menu.menu}
  </div>;
}
