import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import type { EventBadge } from '../../charts/badges';
import { drawLine, placeEndLabels } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import LineSwatch from '../../charts/LineSwatch';
import { lineDashes, lineStyles } from '../../charts/palette';
import { clipToSpans, extentOf, lineEndHour, runAt, stretchAt, stretchStarts } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { lookPlan, useCountryLooks } from '../../dashboard/countryLooks';
import { nameKey, useCountryNamesRead } from '../../dashboard/countryNamesAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import type { NameAt } from '../../dashboard/countryFacts';
import { countryLineColours } from '../../dashboard/countryLineColours';
import { countryCapitulationBadges, countryCapitulationRows } from '../../dashboard/countryCapitulations';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { membershipChanges, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour, usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryStyle, ExperienceReply } from '../../generated/dashboard';
import { definition, OPTIONS } from './definition';
import { BRANCHES, BRANCH_LABELS, experienceHolderAt, experienceHolderCount, experienceHolderEnd, experienceHolders, experienceMembership, experienceName, experienceRaw, experienceRuns, experienceSelection, experienceTooltip, formatExperience, validateExperienceReply } from './experienceData';
import type { ExperienceMeasure } from './experienceData';
import './experience.css';

const NO_WARNINGS: string[] = [];
const NO_NAMES: readonly NameAt[] = [];
const NO_FLAGS: readonly string[] = [];
const END_LABEL_ROOM = 128;
/** Comparison labels also name the branch. */
const COMPARE_LABEL_ROOM = 184;
/** With several countries, colour names the country and the dash the branch. */
const branchDash = (branch: typeof BRANCHES[number]) => lineDashes(BRANCHES.indexOf(branch)).primary;

/** Banked experience, accepted daily gain or net change, with each branch shown. */
export default function ExperienceWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = options.measure as ExperienceMeasure;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-experience',
    params: { branchId: track.branch.id, tags: resolved.tags } } : null;
  const { data, error, retry } = useProduct<ExperienceReply>(runtime.client, request, active, validateExperienceReply);
  const selectedSeries = useMemo(() => experienceSelection(data, resolved.tags, widget.countries.kind === 'group'),
    [data, resolved.tags, widget.countries.kind]);
  const tags = useMemo(() => data ? selectedSeries.map(series => series.tag) : resolved.tags, [data, selectedSeries, resolved.tags]);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = Math.floor(playhead ?? extent[1]);
  const identities = useMemo(() => new Map(selectedSeries.map(series => [series.tag,
    { reads: series.reads, starts: stretchStarts(series.reads) }])), [selectedSeries]);
  const segmentAt = useCallback((tag: string, hour: number) => {
    const identity = identities.get(tag);
    return identity ? stretchAt(identity.reads, identity.starts, hour) : null;
  }, [identities]);
  const namePlan = useMemo(() => lookPlan(track?.branch, tags, head, segmentAt), [track?.branch, tags, head, segmentAt]);
  const names = useCountryNamesRead(runtime.client, track, namePlan.wanted, active, namePlan.maxHours);
  const namePlanKey = JSON.stringify(namePlan.wanted);
  // Stable inside one identity-bounded sample, independent of the playhead.
  const nameHours = useMemo(() => new Map(namePlan.wanted.map(item => [item.tag, item.hour])), [namePlanKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const looks = useCountryLooks(runtime.client, track, tags, head, active, segmentAt, undefined, false, NO_FLAGS);
  const styleValues = selectedSeries.map(item => ({ tag: item.tag, name: null, color: looks.color(item.tag) }));
  const styleKey = JSON.stringify(styleValues);
  // Colour adjustment runs only when the actual style set changes.
  const styles = useMemo(() => new Map<string, CountryStyle>(styleValues.map(item => [item.tag, item])), [styleKey]); // eslint-disable-line react-hooks/exhaustive-deps
  const playback = options.playback as PlaybackMode;
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const branches = BRANCHES.filter(branch => options[branch] === true);
  const single = useMemo(() => experienceHolderCount(selectedSeries) === 1, [selectedSeries]);
  const branchStyles = useMemo(() => lineStyles(BRANCHES.map((key, preferred) => ({ key, preferred })), tokens.categories), [tokens.categories]);
  const tipScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, extent]);
  const tipOwner = useMemo(() => ({}), [runtime.client, tipScope]);
  const [tip, setTip] = useState<{ owner: object; hour: number; events: readonly EventBadge[] } | null>(null);
  const tipKey = useRef<{ owner: object; key: string } | null>(null);
  const currentTip = tip?.owner === tipOwner ? tip : null;
  const showTip = (hour: number | null, events: readonly EventBadge[] = []) => {
    const key = hour === null ? '' : JSON.stringify([hour, events.map(event => [event.hour, event.capitulationTag])]);
    if (tipKey.current?.owner === tipOwner && tipKey.current.key === key) return;
    tipKey.current = { owner: tipOwner, key };
    setTip(hour === null ? null : { owner: tipOwner, hour, events });
  };
  const endKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tags, measure, branches, active, single, options.endLabels]);
  const endOwner = useMemo(() => ({}), [runtime.client, endKey]);
  const currentEndOwner = useRef(endOwner);
  currentEndOwner.current = endOwner;
  const [endAsk, setEndAsk] = useState<{ owner: object; wanted: readonly NameAt[] } | null>(null);
  const endWanted = endAsk?.owner === endOwner ? endAsk.wanted : NO_NAMES;
  const askedEnds = useRef<{ owner: object; key: string } | null>(null);
  const askEndNames = (items: readonly NameAt[]) => {
    const key = JSON.stringify(items);
    if (askedEnds.current?.owner === endOwner && key === askedEnds.current.key) return;
    askedEnds.current = { owner: endOwner, key };
    queueMicrotask(() => { if (currentEndOwner.current === endOwner) setEndAsk({ owner: endOwner, wanted: items }); });
  };
  const endNames = useCountryNamesRead(runtime.client, track, endWanted, active && options.endLabels === true && !single,
    Math.max(MAX_NAME_HOURS, new Set(endWanted.map(item => item.hour)).size));
  useEffect(() => { if (!active) showTip(null); }, [active]); // eslint-disable-line react-hooks/exhaustive-deps
  const menu = useContextMenu('Experience actions');
  useEffect(() => {
    const uncoloured = tags.filter(tag => !styles.get(tag)?.color);
    const warnings = [...(data?.warnings ?? []), ...(looks.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(groups.error ? [`Capitulation markers are unavailable: ${groups.error}`] : []),
      ...(uncoloured.length ? [`Map colours are loading or unavailable for ${uncoloured.join(', ')}; shown in palette colours.`] : []),
      ...(resolved.dropped ? [`${resolved.dropped} more countries are left out; this widget draws at most ${resolved.tags.length}.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, tags, looks.warnings, styles, groups.data, groups.error, resolved.dropped, resolved.tags.length, onDiagnostics]);

  const colours = useMemo(() => countryLineColours(tags, styles,
    tokens.surface, tokens.countryColours === 'fixed'), [tags, styles, tokens.surface, tokens.countryColours]);
  const countries = useMemo(() => {
    if (!data) return [];
    return selectedSeries.map(item => {
      const membership = experienceMembership(resolved.membership, item.tag);
      const inRange = clipToSpans(item.reads, [{ firstHour: extent[0], lastHour: extent[1] }]);
      return { tag: item.tag, reads: membership ? clipToSpans(inRange, membership) : inRange,
        identities: item.identities, readHours: item.reads.map(read => read.firstHour), recorded: item.reads.length > 0, membership,
        name: experienceName(item.tag, nameHours.has(item.tag) ? names.get(nameKey(nameHours.get(item.tag)!, item.tag)) : null),
        color: colours.get(item.tag)!,
        colours: Object.fromEntries(BRANCHES.map(branch => [branch, single ? branchStyles.get(branch)!.color : colours.get(item.tag)!])) as Record<typeof BRANCHES[number], string> };
    });
  }, [data, selectedSeries, colours, branchStyles, names, nameHours, resolved.membership, extent[0], extent[1], single]); // eslint-disable-line react-hooks/exhaustive-deps
  const holders = useMemo(() => experienceHolders(countries), [countries]);
  const headCountries = useMemo(() => holders.map(holder => experienceHolderAt(holder, head)), [holders, head]);
  const wanted = useMemo(() => {
    if (!currentTip) return [];
    const reads = [...holders.flatMap(holder => {
      const country = experienceHolderAt(holder, currentTip.hour);
      return country.identityKnown ? [{ tag: country.tag, hour: currentTip.hour }] : [];
    }), ...currentTip.events.flatMap(event => event.capitulationTag === undefined ? []
      : [{ tag: event.capitulationTag, hour: event.hour }])];
    return [...new Map(reads.map(read => [nameKey(read.hour, read.tag), read])).values()];
  }, [holders, currentTip]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  const lines = useMemo(() => data ? holders.flatMap(holder => BRANCHES.filter(branch => options[branch] === true).map(branch => {
    const parts = holder.parts.map(country => ({ country, runs: experienceRuns(country.reads, data.scale, branch, measure) }));
    return { id: JSON.stringify([holder.id, branch]), branch, parts,
      dash: single ? lineDashes(0).primary : branchDash(branch),
      runs: parts.flatMap(part => part.runs).sort((a, b) => a.firstHour - b.firstHour) };
  })) : [], [data, holders, single, options.army, options.navy, options.air, measure]);
  const layers = useMemo<ChartLayer[]>(() => lines.map<ChartLayer>(line => ({
    draw: (ctx, frame, chartTokens, state) => {
      const at = Math.min(frame.x[1], state.playhead ?? Infinity);
      let endPart: typeof line.parts[number] | undefined;
      let endHour: number | null = null;
      for (const part of line.parts) {
        const hour = lineEndHour(part.runs, frame.x[0], at);
        if (hour !== null && (endHour === null || hour > endHour)) { endHour = hour; endPart = part; }
      }
      for (const part of line.parts) drawLine(ctx, frame, chartTokens, part.runs, part.country.colours[line.branch],
        { dash: line.dash, until: state.playhead ?? undefined, endDot: part === endPart });
    },
    extent: x => extentOf(line.runs, x[0], Math.min(x[1], until ?? x[1])),
  })), [lines, until]);
  const keyHours = useMemo(() => [...new Set(countries.flatMap(country => country.readHours))].sort((a, b) => a - b), [countries]);
  const badges = useMemo<EventBadge[]>(() => {
    const capitulations = countryCapitulationBadges(groups.data, countries.map(country => ({
      tag: country.tag, identities: country.identities, segments: country.reads, membership: country.membership,
    })), extent);
    return countries.flatMap(country => [...(country.membership
      ? membershipChanges(country.membership.map(run => ({ ...run, tag: country.tag })), extent[0], extent[1], resolved.unobserved).map(change => ({
        hour: change.hour, glyph: change.change === 'joined' ? '+' : '−', label: `Country ${change.change}`, color: country.color })) : []),
    ...(capitulations.get(country.tag) ?? []).map(badge => ({ ...badge, color: country.color }))]);
  }, [countries, groups.data, extent[0], extent[1], resolved.unobserved]);

  if (!track) return <WidgetState kind="empty" icon="sparkle" message="Load a campaign to see experience." />;
  if ((widget.time.mode === 'pin' || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading experience…" />;
  if (!countries.some(country => country.recorded)) return <WidgetState kind="unavailable" message="Not in this recording" />;
  if (!branches.length) return <WidgetState kind="empty" message="Turn on an experience branch in the widget settings." />;
  if (measure === 'gain' && !countries.some(country => country.reads.some(read => read.gain !== null)))
    return <WidgetState kind="unavailable" message={`Daily gain is not recorded here. It requires ${data.dayHours} fully observed hours for the same country.`} />;

  return <div className="experience">
    <Legend label={single ? 'Branches' : 'Countries'} items={single ? branches.map(branch => {
      const country = headCountries.find(country => runAt(country.reads, head) !== null);
      const read = country ? runAt(country.reads, head) : null;
      const raw = read ? experienceRaw(read, branch, measure) : null;
      return { id: branch, label: BRANCH_LABELS[branch], color: branchStyles.get(branch)!.color, value: raw == null ? measure === 'gain' ? 'Not recorded' : 'Not known' : formatExperience(raw, data.scale, measure) };
    }) : headCountries.map((country, index) => ({ id: holders[index].id,
      label: country.identityKnown ? country.name : experienceName(country.tag, null), color: country.color,
      value: country.recorded ? undefined : 'Not in this recording' }))} />
    {!single && <p className="chart-line-key">{branches.map(branch => <span key={branch}>
      <LineSwatch dash={branchDash(branch)} />{BRANCH_LABELS[branch]}
    </span>)}</p>}
    <div className="experience-chart">
      <TimeChart label={`Experience of ${headCountries.map(country => country.identityKnown ? country.name : 'Country name unavailable').join(', ')}`} extent={extent} playhead={playhead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays) * 24}
        hoursPerSecond={hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }} fitValues={shown => shown}
        padding={{ right: options.endLabels ? (single ? END_LABEL_ROOM : COMPARE_LABEL_ROOM) : 14 }} layers={layers} keyHours={keyHours} badges={badges}
        active={active} revision={{ head, data, lines, viewport, labels: labels.revision, endNames }}
        onHover={hour => { if (hour === null) showTip(null); }}
        tooltip={(hour, _frame, hovered) => {
          if (hour < extent[0] || hour > extent[1]) { showTip(null); return null; }
          showTip(hour, hovered);
          const shown = holders.map(holder => experienceHolderAt(holder, hour));
          return experienceTooltip(data, shown, branches, measure, hour, labels.labelAt(hour), viewport,
            countryCapitulationRows(hovered, labels.labelAt));
        }}
        overlay={(ctx, frame, chartTokens) => {
          if (!options.endLabels) return;
          const at = Math.min(until ?? extent[1], frame.x[1], extent[1]);
          const wanted: NameAt[] = [];
          const ends = lines.flatMap(line => {
            const end = experienceHolderEnd(line.parts, line.branch, measure, frame.x[0], at,
              (hour, tag) => endNames.get(nameKey(hour, tag)));
            if (!end) return [];
            if (!single) wanted.push({ tag: end.tag, hour: end.hour });
            return [{ id: line.id, y: frame.toY(Number(end.raw) / data.scale), color: end.country.colours[line.branch],
              text: `${single ? BRANCH_LABELS[line.branch] : `${end.name} · ${BRANCH_LABELS[line.branch]}`} ${formatExperience(end.raw, data.scale, measure)}` }];
          });
          askEndNames([...new Map(wanted.map(item => [nameKey(item.hour, item.tag), item])).values()]);
          placeEndLabels(ctx, frame, chartTokens, ends);
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
