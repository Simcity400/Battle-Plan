import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { failureDetails } from '../../dashboard/queryError.ts';
import { useCallback, useMemo,  useRef, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import {  pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import HeatMatrix from '../../charts/heat/HeatMatrix';
import type { MatrixAxisItem, MatrixCell } from '../../charts/heat/HeatMatrix';
import { signedColor } from '../../charts/heat/matrix';
import { dayLabel } from '../../charts/draw';
import { lineSummaryLayer } from '../../charts/renderTimeChart';
import { jumpItem } from '../../charts/jump';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer, ChartData } from '../../charts/TimeChart';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { SECONDARY_DASH } from '../../charts/palette';
import { mix } from '../../shell/appearance/color';
import { mapLive } from '../../charts/live';
import type { LiveValues } from '../../charts/live';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { useHolderLooks } from '../../dashboard/holderLooks.ts';
import { nameReadMetadata, nameReadFailure } from '../../dashboard/nameReads.ts';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt } from '../../dashboard/factionNames';
import { useFactionDisplayNames } from '../../dashboard/factionChoice';
import { noMembersAt, resolveCountries } from '../../dashboard/groups';
import NoMembersState from '../../dashboard/NoMembersState';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { SPEC, type GatedHour } from '../../dashboard/regionTime.ts';
import type { Gate } from '../../dashboard/regionGate';
import { keyId } from '../../dashboard/keyedSeriesData';
import { useKeyedSeries } from '../../dashboard/keyedSeries';
import { contentKey, resolveOptions, setToken } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS, PAIR } from './definition';
import { useTopCountries } from './topCountries.ts';
import { opinionDomain, opinionIndex, opinionText, orderCountries, pairHolders, heldPairKeys, pairKeys, pairOf, pairToken, readHourOf, relationsStructure,
  validateRelations } from './relationsData';
import type { ChosenPair, OpinionDomain } from './relationsData';
import './diplomaticRelations.css';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';

const NO_TAGS: readonly string[] = [];
const NO_WARNINGS: string[] = [];
const OPINIONS = { kind: 'relation-opinions', measure: 'opinion' } as const;
const NO_DOMAIN: OpinionDomain = { low: 0, high: 0, settled: false };

function PairHistory({ runtime, gate, track, pair: chosen, hour, active, following }: Pick<WidgetProps, 'runtime' | 'active'> & {
  gate: Gate; track: NonNullable<ReturnType<WidgetProps['runtime']['tracks']['find']>>; pair: ChosenPair; hour: GatedHour | null; following: boolean;
}) {
  const clock = runtime.clock, axisOffset = track.axisOffset;
  const liveHead = useMemo(() => following ? { read: () => clock.getSnapshot().position + axisOffset, subscribe: (changed: () => void) => clock.subscribeFrame(changed) }
    : undefined, [following, clock, axisOffset]);
  const root = useRef<HTMLDivElement>(null);
  const identities = gate.groups.data?.countryIdentities;
  const held = useMemo(() => identities ? heldPairKeys(identities, chosen) : null, [identities, chosen]);
  const keys = useMemo(() => held?.keys ?? pairKeys(chosen), [held, chosen]);
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const series = useKeyedSeries(runtime.client, runtime.clock, gate, { source: track.source, branch: track.branch, family: OPINIONS, keys, partBits: 32, lastHour: track.branch.lastHour, ...(held ? { within: held.within } : {}) },
    hour, gate.recorded(hoverHour), false, active, root);
  const looks = useHolderLooks(runtime.client, track, useMemo(() => pairHolders(chosen), [chosen]), active, useChartTokens());
  const label = (index: number) => looks.name(index ? 'second' : 'first');
  const colour = (index: number) => looks.colour(index ? 'second' : 'first');
  const layers = useMemo<ChartLayer[]>(() => keys.map((key, index) => lineSummaryLayer({
    summary: () => series.summaryOf(keyId(key)),
    color: looks.colour(index ? 'second' : 'first'),
    held: true,
    options: index ? { dash: SECONDARY_DASH } : {},
    withExtent: false,
  })), [keys, series.summaryOf, looks]);
  const chartData = useMemo<ChartData>(() => ({ revision: () => [series.revision()], subscribe: series.subscribe }), [series.revision, series.subscribe]);
  const valueAt = (index: number, at: number) => {
    const line = series.lines.find(item => keyId(item.key) === keyId(keys[index]));
    const run = line?.runs.find(item => item.firstHour <= at && at <= item.lastHour);
    return run ? opinionText(Number(run.valueRaw)) : 'Not recorded';
  };
  if (series.error) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
  return <div ref={root} className="relations-pair">
    <TimeChart label={`${label(0)} and ${label(1)} opinions over time`} extent={[track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)]}
      playhead={hour} liveHead={liveHead} playback="cursor" branchStart={track.branch.firstHour} axes={{ xFrom: null, xTo: null, yMin: null, yMax: null, log: false }}
      layers={layers} active={active} notRecorded={series.unrecorded} revision={`${pairToken(chosen)}:${series.summaryByKey.size}:${looks.revision}`} data={chartData}
      onSummaryView={series.onSummaryView} onHover={setHoverHour}
      tooltip={at => ({ title: dayLabel(at), rows: [
        { label: `${label(0)}’s opinion of ${label(1)}`, value: valueAt(0, Math.floor(at)), swatch: colour(0) },
        { label: `${label(1)}’s opinion of ${label(0)}`, value: valueAt(1, Math.floor(at)), swatch: colour(1) },
      ] })} />
  </div>;
}

export default function DiplomaticRelationsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const displayNames = useFactionDisplayNames(track?.source);
  const gate = useGate(runtime, widget, SPEC, active, { members: true });
  const hour = gate.hour;
  const groups = useCountryGroups(runtime, track, active);
  const resolved = track ? resolveCountries(widget.countries, [defaultCountryKey(track) ?? ''].filter(Boolean), groups.data, track.branch.lastHour, Number.MAX_SAFE_INTEGER) : null;
  const top = resolved?.top ?? null;
  const [rankRoot, setRankRoot] = useState<HTMLDivElement | null>(null);
  const ranked = useTopCountries({ widget, runtime, active }, gate, track, top, options.topMeasure, useMemo(() => ({ current: rankRoot }), [rankRoot]));
  const includes = gate.members?.includes;
  const members = useMemo(() => hour === null || !includes ? NO_TAGS : (resolved?.tags ?? []).filter(includes),
    [resolved?.tags.join(','), includes, hour]);
  const tags = top === null ? members : ranked.tags ?? NO_TAGS;
  const opinions = useHeadProduct(runtime, gate, 'diplomatic-relations', track && (top === null || ranked.tags) && tags.length > 1 ? { countries: tags.map(countryFromKey) } : null,
    { active, validate: validateRelations, structure: relationsStructure });
  const data = opinions.data;
  const shownAt = opinions.at ?? hour;
  const shown = shownAt ?? Number.NaN;
  const live = opinions.live;
  useShownEachFrame(clock, () => data ? opinions.shownAt() : null, opinions.live);
  const [visibleTags, setVisibleTags] = useChangedState<readonly string[]>([]);
  const scope = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const [choice, setChoice] = useState<{ scope: object; pair: ChosenPair } | null>(null);
  const picked = choice?.scope === scope ? choice.pair : null;
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => [...(shownAt === null ? [] : tags.map(tag => ({ hour: shownAt, tag }))),
    ...(picked ? pairHolders(picked).map(holder => ({ hour: holder.hour, tag: holder.tag })) : [])], [tags, shownAt, picked]), active);
  const flagsWanted = useMemo(() => shownAt === null ? [] : visibleTags.map(tag => ({ hour: shownAt, tag })), [visibleTags, shownAt]);
  const flags = useCountryLabelsAt(runtime.client, track, flagsWanted, active);
  const label = useCallback((tag: string, at = shown) => nameReadMetadata(names.get(nameKey(at, tag))), [names, shown]);
  const ordered = useMemo(() => orderCountries(tags, options.order === 'name' ? 'name' : 'faction', tag => label(tag).name, groups.data, shown,
    id => { const faction = groups.data?.factions.find(item => item.id === id); return faction ? factionNameAt(faction, shown, displayNames[id]).name ?? id : id; }), [tags, options.order, label, groups.data, shown, displayNames]);
  const axis = useMemo<MatrixAxisItem[]>(() => ordered.map(tag => { const country = flags.labelAt(shown)(tag); const named = label(tag);
    return { id: tag, label: named.name, countryFlag: { ...(named.nameFailure ? { nameFailure: named.nameFailure } : {}), flag: country.flag, flagReason: country.flagReason, ...(country.flagDetail !== undefined ? { flagDetail: country.flagDetail } : {}), flagPending: country.flagPending } }; }), [ordered, flags, label, shown]);
  const domainOf = useMemo(() => {
    let held: OpinionDomain | null = null;
    return () => data ? held = opinionDomain(data, live.read(), held) : NO_DOMAIN;
  }, [data, live]);
  const { low, high } = useSyncExternalStore(live.subscribe, domainOf);
  const grey = mix(tokens.surface, tokens.text, 0.2);
  const look = useCallback((values: LiveValues<number>) => {
    const lookup = data ? opinionIndex(data, values) : null;
    return (owner: string, target: string): MatrixCell => {
      const found = lookup?.(owner, target) ?? null;
      if (!found) return null;
      return 'unknown' in found ? { unknown: true } : { color: signedColor(Math.max(low, Math.min(high, found.value)), low, high, grey, tokens.error, tokens.positive) };
    };
  }, [data, low, high, grey, tokens.error, tokens.positive]);
  const cells = useMemo(() => mapLive(live, (values): LiveValues<MatrixCell> => {
    const at = look(values);
    return { get: key => { const split = key.indexOf('\u0000'); return at(key.slice(0, split), key.slice(split + 1)); }, missing: () => 'Not recorded' };
  }), [live, look]);
  const cell = useCallback((row: number, column: number): MatrixCell => look(live.read())(ordered[row], ordered[column]), [look, live, ordered]);
  const tooltip = useCallback((row: number, column: number): TooltipContent | null => {
    const [a, b] = [ordered[row], ordered[column]];
    const values = live.read(), lookup = data ? opinionIndex(data, values) : null;
    const there = lookup?.(a, b), back = lookup?.(b, a);
    const text = (found: typeof there) => !found ? 'Not recorded' : 'unknown' in found ? found.reason : opinionText(found.value);
    const read = there && !('unknown' in there) ? readHourOf(values, a) : null;
    return { ...countryNameDetails(label(a), label(b)), title: `${label(a).name}’s opinion of ${label(b).name}: ${text(there)}`, subtitle: dayLabel(shown), rows: [
      { label: `${label(b).name}’s opinion of ${label(a).name}`, value: text(back) },
      ...(read != null && read !== shown ? [{ label: 'As of', value: dayLabel(read) }] : []),
    ] };
  }, [ordered, live, data, label, shown]);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const savedPairKey = typeof options.pair === 'string' && options.pair ? contentKey(options.pair) : null;
  const savedPair = pairOf(savedPairKey);
  const anchor = useRef<HTMLDivElement>(null);
  const diagnostics = useMemo(() => !data ? (opinions.error ? [opinions.error.message] : NO_WARNINGS) : data.warnings, [data, opinions.error]);
  useDiagnostics(onDiagnostics, diagnostics);
  const drillOpen = drill.breakdown != null && picked != null;
  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const body = (() => {
    if (!track) return <WidgetState kind="empty" icon="handshake" message="Load a campaign to see opinions." />;
    const groupFailure = groups.error && widget.countries.kind === 'group' ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
    if (options.show === PAIR.id) {
      if (groupFailure && !groups.data) return groupFailure;
      if (resolved?.reason) return <>{groupFailure}<WidgetState kind="empty" message={resolved.reason} /></>;
      return <>{groupFailure}{savedPair
        ? <PairHistory runtime={runtime} gate={gate} track={track} pair={savedPair} hour={hour} active={active} following={widget.time.mode === 'follow'} />
        : <WidgetState kind="empty" message="Choose a pair: click a cell in a Diplomatic relations matrix and pin it." />}</>;
    }
    const none = resolved && hour !== null ? noMembersAt(widget.countries, resolved, groups.data, hour, track.branch.lastHour) : null;
    const state = opinions.outside || hour === null ? <WidgetState kind="outside" read={opinions} />
      : groupFailure && !groups.data ? groupFailure
      : resolved?.reason ? <>{groupFailure}<WidgetState kind="empty" message={resolved.reason} /></>
      : none ? <>{groupFailure}<NoMembersState reason={none.reason} icon="handshake" /></>
      : ranked.unavailable ? <>{groupFailure}<WidgetState kind="unavailable" technical={ranked.unavailable} /></>
      : top !== null && ranked.error ? <>{groupFailure}<WidgetState kind="error" what="the top countries" failure={ranked.error} onRetry={ranked.retry} /></>
      : top !== null && !ranked.tags ? <>{groupFailure}<WidgetState kind="loading" label="Ranking countries…" /></>
      : tags.length < 2 ? <>{groupFailure}<WidgetState kind="empty" icon="handshake" message="Choose at least two countries." /></>
      : opinions.error ? <>{groupFailure}<WidgetState kind="error" failure={opinions.error} onRetry={opinions.retry} /></>
      : !data ? <>{groupFailure}<WidgetState kind="loading" label="Loading opinions…" /></> : null;
    if (state && !drillOpen) return state;
    const idOf = (tag: string) => data?.owners.find(owner => owner.tag === tag)?.countryId ?? null;
    const main = state ?? (data && <HeatMatrix label="Opinions" rows={axis} columns={axis} cell={cell} live={cells} revision={`${ordered.join(',')}:${low}:${high}:${grey}:${JSON.stringify(data.owners)}`} active={active && (!picked || options.drill === 'popout')}
      tooltip={tooltip} onVisible={(rows, columns) => setVisibleTags(previous => { const next = [...new Set([...rows, ...columns])];
        return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })}
      onSelect={(row, column) => {
        const [first, second] = [ordered[row], ordered[column]];
        const [firstId, secondId] = [idOf(first), idOf(second)];
        if (firstId === null || secondId === null || shownAt === null) return;
        setChoice({ scope, pair: { first, second, hour: shownAt, firstId, secondId } });
        dispatch({ type: 'open', breakdown: PAIR.id, hour: shownAt, follow: false });
      }}
      menu={(row) => { const read = readHourOf(live.read(), ordered[row]); return read != null && read !== shown ? [jumpItem(runtime, track.id, read)] : []; }}
      legend={[{ id: 'low', label: low < 0 ? opinionText(low) : '0', color: low < 0 ? tokens.error : grey }, { id: 'zero', label: '0', color: grey },
        { id: 'high', label: high > 0 ? opinionText(high) : '0', color: high > 0 ? tokens.positive : grey }]} />);
    return <div ref={element => { anchor.current = element; opinions.root(element); }} className="diplomatic-relations" data-detail={failureDetails([...names.values()].map(nameReadFailure))}>{state ? null : groupFailure}
      <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} mode={options.drill === 'popout' ? 'popout' : 'inline'} title="Pair history" day={drillControl.hour === null ? '' : dayLabel(drillControl.hour)} canFollow={false} anchor={anchor} onClose={() => { dispatch({ type: 'close' }); setChoice(null); }}
        pinRefused={pinTimed.refused ?? null} onPin={picked && pinTime ? () => { const pinned = pinnedBreakdown(widget, PAIR, track.id, pinTime);
          runtime.addWidgetBelow(widget.id, { ...pinned, title: `${label(picked.first, picked.hour).name} and ${label(picked.second, picked.hour).name} opinions`, options: { ...pinned.options, pair: setToken(pairToken(picked)) } }); } : undefined}
        main={main}>
        {picked && <PairHistory runtime={runtime} gate={drillControl.gate} track={track} pair={picked} hour={drillControl.hour} active={active} following={drillControl.following} />}
      </DrillHost>
    </div>;
  })();
  return <div ref={setRankRoot} className="relations-root">{body}</div>;
}
