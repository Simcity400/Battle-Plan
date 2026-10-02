import { useCountryColours } from '../../charts/race/countryColours';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, drawBands, drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import type { RaceItem } from '../../charts/race/race';
import RaceChart from '../../charts/race/RaceChart';
import type { RaceView } from '../../charts/race/options';
import { mixCountry } from '../../charts/race/options';
import { linesAt, linesTooltip } from '../../charts/race/lines';
import { extentOfAll, lineLabelHours, runAt } from '../../charts/series';
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
import { contentSet, resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryFactoriesReply } from '../../generated/dashboard';
import { MAX_COUNTRIES, OPTIONS } from './definition';
import { changeHours, chosenGroups, factoriesAt, factoriesRequest, factoryModel, factorySegmentAt, factoryTooltips, formatCount, groupName, lineRuns, mixBands, raceEntries, rankFactories, typeColours,
  validateFactoriesReply } from './factoriesData';
import type { CountMode } from './factoriesData';
import './factories.css';

const NO_WARNINGS: string[] = [];
/** Room for the names at the lines' ends. */
const END_LABEL_ROOM = 150;

/** Factory counts per country (spec 4.2 Factories): a race re-ranked every hour, the
 * same counts as lines over time, or one country's factories stacked by type. */
export default function FactoriesWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view = (options.view as RaceView) ?? 'race';
  const mode = (options.count as CountMode) ?? 'free';
  const chosen = contentSet(options.types);
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
  const params = track ? factoriesRequest(track.branch.id, resolved, mode, chosen) : null;
  const request = track && params && (params.top || params.tags.length)
    ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-factories', params } : null;
  const { data, error, retry } = useProduct<CountryFactoriesReply>(runtime.client, request, active, validateFactoriesReply);
  const model = useMemo(() => data ? factoryModel(data) : null, [data]);
  const groups = useMemo(() => model ? chosenGroups(model, chosen) : [], [model, chosen.join(' ')]); // eslint-disable-line react-hooks/exhaustive-deps
  const slots = resolved.top ?? Math.max(1, resolved.tags.length);
  // The hours a country is shown: in the top N (from the product), or a member of a
  // group counted at each hour; null shows it whenever recorded.
  const membershipOf = (tag: string): readonly Span[] | null => model?.series.get(tag)?.membership ?? resolved.membership?.get(tag) ?? null;
  // The hours still to cut a country's runs to: a group's, counted at each hour. A
  // top-N reply's runs are already cut to its hours, so they are not cut again.
  const cutTo = (tag: string): readonly Span[] | null => resolved.membership?.get(tag) ?? null;

  // The countries at the moment, ranked by the count shown.
  const ranked = useMemo(() => model ? rankFactories(model, rankedCandidates(resolved, moment, model.tags), moment, groups, mode, slots) : [],
    [model, resolved, moment, groups, mode, slots]);
  // The lines: the chosen countries, or every country ever in the top N, each drawn
  // while it was.
  const lineTags = useMemo(() => resolved.top != null ? model?.tags ?? [] : resolved.tags, [resolved, model]);
  // One country's mix: the first chosen, the dashboard's, or for a group the member
  // ranked first at the moment shown (never a country outside it then).
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const shownTags = view === 'race' ? ranked.map(entry => entry.tag) : view === 'lines' ? lineTags : mixTag ? [mixTag] : [];
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groupReply.data?.warnings ?? []),
      ...(resolved.dropped ? [`${resolved.dropped} more countries are left out; this widget shows at most ${MAX_COUNTRIES}.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groupReply.data, resolved.dropped, onDiagnostics]);

  const split = options.split === true;
  const menu = useContextMenu('Factory actions');
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);

  // The hours shown: a date range, or the branch. Nothing outside it is drawn, fitted
  // or hovered, even where an axis is dragged past it.
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  // Lines over time: one per country, cut to the hours it is shown.
  const lines = useMemo(() => !model || view !== 'lines' ? [] : lineTags.flatMap(tag => {
    const shown = lineRuns(model, tag, groups, mode, cutTo(tag), range);
    return shown.length ? [{ tag, runs: shown }] : [];
  }), [model, view, lineTags, groups, mode, resolved.membership, range]); // eslint-disable-line react-hooks/exhaustive-deps
  const [labelView, setLabelView] = useState<[number, number] | null>(null);
  const onLineView = useCallback((x: readonly [number, number]) => {
    const from = Math.floor(x[0]);
    const to = Math.floor(x[1]);
    setLabelView(before => before?.[0] === from && before[1] === to ? before : [from, to]);
  }, []);
  const endLabels = options.endLabels !== false;
  // A legend names every line, even outside the zoomed view; end labels name
  // only the visible ends. Both use the line's own hour, capped while Live is on.
  const labelHours = useMemo(() => lineLabelHours(lines, endLabels ? Math.max(extent[0], labelView?.[0] ?? extent[0]) : extent[0],
    endLabels ? Math.min(extent[1], labelView?.[1] ?? extent[1]) : extent[1], until), [lines, extent[0], extent[1], labelView, until, endLabels]); // eslint-disable-line react-hooks/exhaustive-deps
  const lookSegment = useCallback((tag: string, hour: number) => {
    return factorySegmentAt(model, tag, hour);
  }, [model]);
  const looks = useCountryLooks(runtime.client, track, shownTags, moment, active, lookSegment, view === 'lines' ? labelHours : undefined);
  const typeColors = useMemo(() => typeColours(model?.reply.groups ?? [], tokens.categories), [model, tokens]);
  const colorOf = useCountryColours(shownTags, looks.color, tokens, looks);

  const items = useMemo<RaceItem[]>(() => raceEntries(ranked, { mode, split, country: colorOf, type: key => typeColors.get(key) ?? tokens.muted }),
    [ranked, mode, split, typeColors, colorOf, tokens.muted]);

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
      const hours = endLabels ? lineLabelHours(lines, frame.x[0], Math.min(frame.x[1], extent[1]), until) : undefined;
      for (const line of lines) drawLine(ctx, frame, chartTokens, line.runs, colorOf(line.tag, hours?.get(line.tag)), { until: state.playhead ?? undefined });
    },
    extent: x => extentOfAll(lines.map(line => line.runs), x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [lines, colorOf, until, extent[1], endLabels]); // eslint-disable-line react-hooks/exhaustive-deps

  // One country's mix: its types stacked, negative counts below zero, cut to the
  // hours it is shown.
  const mixMembership = mixTag ? membershipOf(mixTag) : null;
  const mixCut = mixTag ? cutTo(mixTag) : null;
  const mix = useMemo(() => {
    if (!model || view !== 'mix' || !mixTag) return null;
    const bands = mixBands(model, mixTag, groups, mode, mixCut, range);
    const hours = [...new Set(bands.flatMap(runs => runs.map(run => run.firstHour)))].sort((a, b) => a - b);
    return { stack: buildStack(bands), hours };
  }, [model, view, mixTag, groups, mode, mixCut, range]);
  const mixLayers = useMemo<ChartLayer[]>(() => mix ? [{
    draw: (ctx, frame, chartTokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of stackedSegments(mix.stack, frame.x[0], to, hoursPerPixel / 2)) {
        drawBands(ctx, frame, chartTokens, segment.hours, segment.bands, groups.map(key => typeColors.get(key) ?? chartTokens.muted));
      }
    },
    extent: x => stackExtent(mix.stack, x[0], Math.min(x[1], until ?? x[1])),
  }] : [], [mix, groups, typeColors, until]);

  if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see its factories." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (grouped && groupReply.error) return <WidgetState kind="error" message={`Country groups are unavailable: ${groupReply.error}`} onRetry={groupReply.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" icon="factory" message={resolved.reason} />;
  // A group's members are not known until its history arrives; nothing is shown as empty meanwhile.
  if (!data || !model || (grouped && !groupReply.data)) return <WidgetState kind="loading" label="Loading factories…" />;
  if (!model.tags.length) return <WidgetState kind="unavailable" message="No factories were recorded for these countries in this branch." />;

  const modeLabel = OPTIONS.find(field => field.key === 'count')!;
  const countName = modeLabel.kind === 'choice' ? modeLabel.choices.find(choice => choice.value === mode)?.label ?? mode : mode;
  // `name` names the country as recorded at the hour the tooltip is about.
  const tooltipAt = (tag: string, hour: number, name: string) => {
    const at = isMember(membershipOf(tag) ?? undefined, hour) ? factoriesAt(model, tag, hour, groups, mode) : null;
    return at ? fitTooltip(factoryTooltips(model, name, at, dayLabel(hour), mode, typeColors), viewport, { keep: groups.length > 1 ? 1 : 0 }) : null;
  };
  const axes = { xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: view === 'lines' && options.log === true };
  const jump = (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]);

  if (view === 'race') {
    return <div className="factories">
      {split && groups.length > 1 && <Legend label="Factory types" items={groups.map(key => ({ id: key, label: groupName(model.reply.groups.find(group => group.key === key)!), color: typeColors.get(key) ?? tokens.muted }))} />}
      <div className="factories-chart">
        <RaceChart label={`Factories, ${countName}, ${dayLabel(moment)}`} items={items} slots={slots} name={looks.name} flag={looks.flag}
          tooltip={tag => tooltipAt(tag, moment, looks.name(tag))} onContextMenu={(_, x, y) => jump(moment, x, y)}
          dateOverlay={options.dateOverlay === true ? dayLabel(moment) : null}
          empty={`No factories recorded on ${dayLabel(moment)}.`} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond}
          active={active} resetKey={`${track.source.id}:${track.branch.id}`} />
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
    if (!lines.length) return <WidgetState kind="empty" icon="factory" message="No countries to draw. Choose countries in the widget settings." />;
    return <div className="factories">
      {!endLabels && <Legend label="Countries" items={lines.map(line => ({ id: line.tag, label: looks.name(line.tag), color: colorOf(line.tag) }))} />}
      <div className="factories-chart" onPointerLeave={() => showTip(null)}>
        <TimeChart {...common} label={`Factories over time, ${countName}`} layers={lineLayers} keyHours={lineHours}
          onViewChange={onLineView} padding={{ right: endLabels ? END_LABEL_ROOM : 14 }} revision={JSON.stringify([lines.length, looks.revision, labels.revision, viewport])}
          tooltip={hour => {
            if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
            showTip(hour, linesAt(hour, lines));
            return fitTooltip([linesTooltip(hour, lines, labels.labelAt, colorOf, dayLabel(hour), countName, formatCount)], viewport);
          }}
          overlay={(ctx, frame, chartTokens) => {
            if (!endLabels) return;
            const ends = lines.flatMap(line => {
              const last = lineLabelHours([line], frame.x[0], Math.min(frame.x[1], extent[1]), until).get(line.tag);
              const run = last == null ? null : runAt(line.runs, last);
              return run ? [{ id: line.tag, color: colorOf(line.tag, last!), y: frame.toY(run.value), text: `${looks.name(line.tag, last!)} ${formatCount(run.value)}` }] : [];
            });
            placeEndLabels(ctx, frame, chartTokens, ends);
          }} />
      </div>
      {menu.menu}
    </div>;
  }

  if (!mixTag || !mix) return <WidgetState kind="empty" icon="factory"
    message={widget.countries.kind === 'group' ? `No country of this group was recorded on ${dayLabel(moment)}.` : 'Choose a country in the widget settings.'} />;
  if (!mix.hours.length) return <WidgetState kind="unavailable" message={`No factories were recorded for ${looks.name(mixTag)} in this branch.`} />;
  const mixAt = isMember(mixMembership ?? undefined, moment) ? factoriesAt(model, mixTag, moment, groups, mode) : null;
  return <div className="factories">
    <p className="factories-country">{looks.name(mixTag)}</p>
    <Legend label="Factory types" items={groups.map(key => {
      const count = mixAt?.groups.find(group => group.key === key)?.count;
      return { id: key, label: groupName(model.reply.groups.find(group => group.key === key)!), color: typeColors.get(key) ?? tokens.muted,
        value: count == null ? '—' : formatCount(count) };
    })} />
    <div className="factories-chart" onPointerLeave={() => showTip(null)}>
      <TimeChart {...common} label={`Factories of ${looks.name(mixTag)} by type, ${countName}`} layers={mixLayers} keyHours={mix.hours}
        revision={JSON.stringify([mixTag, mix.hours.length, viewport, groups, labels.revision])}
        tooltip={hour => {
          if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
          showTip(hour, [mixTag]);
          return tooltipAt(mixTag, hour, labels.labelAt(hour)(mixTag).name);
        }} />
    </div>
    {menu.menu}
  </div>;
}
