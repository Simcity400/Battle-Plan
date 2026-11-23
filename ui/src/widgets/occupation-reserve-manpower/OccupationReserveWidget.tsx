import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { CountryRacePart, LiveRaceEntries } from '../../charts/race/countryRace';
import { useCountryColours } from '../../charts/race/countryColours';
import DrillHost from '../../charts/drill/DrillHost';
import {  pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { mapLive } from '../../charts/live';
import type { LiveHead } from '../../charts/TimeChart';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter, noMembersAt, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { shownHour } from '../../dashboard/IntelCountry';
import NoMembersState from '../../dashboard/NoMembersState';
import { contentKey, resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { BREAKDOWN, OPTIONS } from './definition';
import { holderBands, holderOf, holderToken, historySources, holderTooltip, persons, raceEntries, raceHolders, reserveOf, reserveStructure, segmentAt,
  segmentParts, sourceKey, sourceTagAt, UNRESOLVED, validateOccupationReserve, validateReserveHistory } from './occupationReserveData';
import './occupationReserve.css';
import type { ReserveHistoryReply } from '../../generated/dashboard';
import {  SPEC } from '../../dashboard/regionTime';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime';
import { productRequest } from '../../dashboard/request.ts';

const PARAMS = {} as const;
const NO_TAGS: readonly string[] = [];

export default function OccupationReserveWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics } = props;
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const pinned = options.show === 'breakdown';
  const readHour = useRef<number | null>(null);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'occupation-reserve-manpower', track && !pinned ? PARAMS : null,
    { active, validate: validateOccupationReserve, structure: reserveStructure, paint: reply => { readHour.current = reply.readHour; } });
  useShownEachFrame(runtime.clock, head.shownAt, head.live);
  const data = head.data;
  if (data && head.read !== 'followed') readHour.current = data.readHour;
  const moment = head.at ?? shownHour(runtime, widget, head) ?? track?.branch.lastHour ?? 0;
  const following = widget.time.mode === 'follow';
  const playhead = following || widget.time.mode === 'pin' ? moment : null;
  const extent: [number, number] = track ? [track.branch.firstHour, Math.max(track.branch.firstHour + 1, track.branch.lastHour)] : [0, 1];
  const liveHead = useMemo<LiveHead | undefined>(() => following && track ? { read: () => runtime.clock.getSnapshot().position + track.axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, track, runtime.clock]);
  const all = widget.countries.kind === 'all';
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const manual = widget.countries.kind === 'tags' || widget.countries.kind === 'selected';
  const groups = useCountryGroups(runtime, track, active && (grouped || manual || drill.breakdown !== null || pinned));
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : null, [groups.data]);
  const counted = useMemo(() => (manual || grouped) && identities ? projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, identities, track?.branch.firstHour ?? 0)) : resolved,
  [manual, grouped, identities, resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour]);
  const holders = useMemo(() => {
    if (!data) return [];
    const tags = data.holders.flatMap(holder => holder.country.tag === null ? [] : [holder.country.tag]);
    const candidates = all ? null : new Set(rankedCandidates(counted, moment, tags));
    return raceHolders(data, tag => candidates === null || candidates.has(tag), resolved.top != null);
  }, [data, all, counted, moment, resolved.top]);
  const unnamed = data ? data.holders.filter(holder => holder.country.tag === null && (holder.admitted || holder.entries.length > 0)).length : 0;
  const sourceTags = useMemo(() => [...new Set(holders.flatMap(holder => holder.entries.flatMap(entry => entry.source.tag === null ? [] : [entry.source.tag])))], [holders]);
  const holderTags = useMemo(() => holders.map(holder => holder.country.tag!), [holders]);
  const lookTags = useMemo(() => [...new Set([...holderTags, ...sourceTags])], [holderTags, sourceTags]);
  const looks = useCountryLooks(runtime.client, track, lookTags, moment, active);
  const colour = useCountryColours(sourceTags, looks.color, tokens, looks);
  const sourceColour = useCallback((source: string) => source === UNRESOLVED ? tokens.muted : colour(source, moment), [colour, moment, tokens.muted]);
  const sourceName = useCallback((source: { tag: string | null }) => source.tag === null ? 'Source country unavailable' : looks.name(source.tag, moment), [looks, moment]);
  const parts = useMemo<CountryRacePart[]>(() => [...new Set(holders.flatMap(holder => holder.entries.map(entry => sourceKey(entry.source))))]
    .map(source => ({ id: source, label: source === UNRESOLVED ? 'Source country unavailable' : looks.name(source, moment), color: sourceColour(source) })),
  [holders, looks, moment, sourceColour]);
  const live = head.live;
  const top = resolved.top ?? null;
  const ranked = useMemo(() => mapLive(live, values => raceEntries(holders, values, top, sourceColour, moment)), [live, holders, top, sourceColour, moment]);
  const liveRanked = useMemo<LiveRaceEntries>(() => ({ entries: ranked.read, hour: () => head.shownAt() ?? moment, bind: ranked.subscribe }),
    [ranked, moment]);
  const byTag = useMemo(() => new Map(holders.map(holder => [holder.country.tag!, holder])), [holders]);
  const [chosen, setChosen] = useState<{ tag: string; id: number } | null>(null);
  const pinnedHolder = pinned && options.holder ? holderOf(contentKey(String(options.holder))) : null;
  const drilled = pinned ? pinnedHolder : drill.breakdown ? chosen : null;
  const drillRegion = useRegion(widget, pinned ? SPEC : drillControl.region.action);
  const regionAt = useRegionHour(runtime, drillRegion, active && (pinned || drill.breakdown !== null));
  const drillAt = pinned || drillControl.following && widget.time.mode === 'follow' ? moment : regionAt;
  const headRoot = head.root;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; headRoot(element); }, [headRoot]);
  const none = grouped ? noMembersAt(widget.countries, resolved, groups.data, moment, track?.branch.lastHour ?? 0) : null;
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...unnamed ? [`${unnamed} holders the recording names no tag for are not shown.`] : [], ...groups.data?.warnings ?? []] : null);
  if (!track) return <WidgetState kind="empty" icon="flag" message="Load a campaign to see occupation manpower." />;
  if (pinned) return head.outside ? <WidgetState kind="outside" read={head} /> : <HolderSources {...props} holder={pinnedHolder} hour={moment} extent={extent} playhead={playhead} liveHead={liveHead} />;
  const groupFailure = grouped && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const parentState = head.read === 'off' ? <WidgetState kind="outside" />
    : head.error && !data ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
      : groupFailure && !groups.data ? null
        : resolved.reason && !all ? <WidgetState kind="empty" message={resolved.reason} />
          : none ? <NoMembersState reason={none.reason} icon="flag" />
            : !data || grouped && !groups.data ? <WidgetState kind="loading" label="Loading occupation manpower…" />
              : data.missing ? <WidgetState kind="unavailable" message="Occupation manpower was not recorded at this time." detail={data.missing} /> : undefined;
  const tooltip = (tag: string, _hour: number, name: string): TooltipContent[] | null => {
    const holder = byTag.get(tag);
    return holder ? [holderTooltip(reserveOf(holder, live.read()), name, sourceName, sourceColour, readHour.current === null ? null : dayLabel(readHour.current))] : null;
  };
  const main = parentState !== undefined ? parentState : <CountryRaceView runtime={runtime} track={track} active={active && !drill.breakdown} options={options} view="race" extent={extent}
    playhead={playhead} moment={moment} slots={top ?? Math.max(1, holders.length)} ranked={ranked.read()} liveRanked={liveRanked} lines={[]} parts={parts}
    raceLegendParts={parts} partsLabel="Source countries" countryTags={holderTags} label="Occupation reserve manpower"
    empty="No reserve manpower is held at this date." formatValue={value => Math.round(value).toLocaleString('en-US')}
    labels={{ race: hour => `Occupation reserve manpower, ${dayLabel(hour)}`, actions: 'Occupation reserve actions' }}
    presentation={{ exact: true, windowedLegend: true }} tooltip={tooltip}
    onPick={(hour, tag) => {
      const holder = tag ? byTag.get(tag) : undefined;
      if (!holder) return;
      setChosen({ tag: holder.country.tag!, id: holder.country.id });
      dispatch({ type: 'open', breakdown: BREAKDOWN.id, hour, follow: drillControl.followsAt(hour) });
    }} />;
  return <div ref={root} className="parts-race occupation-reserve">{groupFailure}
    <DrillHost drill={drillControl} runtime={runtime} subject={drilled !== null} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title={drilled ? looks.name(drilled.tag, drillAt ?? moment) : BREAKDOWN.label} day={drillAt === null ? '' : dayLabel(drillAt)} canFollow onClose={() => dispatch({ type: 'close' })} main={main}
      pinRefused={drillControl.pinRefused} onPin={drilled && drillControl.time() ? () => {
        const time = drillControl.time();
        if (!time) return;
        const pin = pinnedBreakdown(widget, BREAKDOWN, track.id, time);
        pin.options = { ...pin.options, holder: setToken(holderToken(drilled)) };
        runtime.addWidgetBelow(widget.id, pin);
      } : undefined}>
      {drilled && <HolderSources {...props} holder={drilled} hour={drillAt ?? moment} extent={extent} playhead={drill.follow ? playhead : drillAt} liveHead={drill.follow ? liveHead : undefined} />}
    </DrillHost>
  </div>;
}

function HolderSources({ widget, runtime, active, holder, hour, extent, playhead, liveHead }: WidgetProps & {
  holder: { tag: string; id: number } | null; hour: number; extent: [number, number]; playhead: number | null; liveHead: LiveHead | undefined;
}) {
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const request = track && holder ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'occupation-reserve-history',
    params: { branchId: track.branch.id, holder: holder.id } }) : null;
  const { data, error, retry } = useProduct<ReserveHistoryReply>(runtime.client, request, active, validateReserveHistory, track?.source);
  const sources = useMemo(() => data ? historySources(data) : [], [data]);
  const bands = useMemo(() => data ? holderBands(data, sources) : [], [data, sources]);
  const tagOf = useCallback((source: number) => data ? sourceTagAt(data, source, hour)
    ?? [...data.sources.find(item => item.id === source)?.tags ?? []].reverse().find(tag => tag.tag !== null)?.tag ?? null : null, [data, hour]);
  const sourceTags = useMemo(() => [...new Set(sources.flatMap(source => { const tag = tagOf(source); return tag === null ? [] : [tag]; }))], [sources, tagOf]);
  const looks = useCountryLooks(runtime.client, track, useMemo(() => holder ? [holder.tag, ...sourceTags] : NO_TAGS, [holder, sourceTags]), hour, active);
  const colour = useCountryColours(sourceTags, looks.color, tokens, looks);
  const segment = data ? segmentAt(data, Math.floor(hour)) : null;
  const values = segment ? segmentParts(segment, sources) : sources.map(() => undefined);
  const mix = holder ? { tag: holder.tag, bands, parts: sources.map((source, index) => {
    const tag = tagOf(source);
    const value = values[index];
    return { id: String(source), label: tag === null ? 'Source country unavailable' : looks.name(tag, hour), color: tag === null ? tokens.muted : colour(tag, hour),
      value: value === undefined ? '—' : persons(value) };
  }) } : null;
  const tooltip = (_tag: string, at: number, name: string): TooltipContent[] | null => {
    const shown = data ? segmentAt(data, Math.floor(at)) : null;
    if (!data || !shown) return null;
    if (!shown.entries) return [{ title: name, rows: [{ label: 'Reserve manpower', value: `Not recorded (${shown.unknown})` }, { label: 'As of', value: dayLabel(shown.firstHour) }] }];
    const named = (source: number) => { const tag = sourceTagAt(data, source, Math.floor(at)); return tag === null ? 'Source country unavailable' : looks.name(tag, at); };
    return [{ title: name, rows: [
      { label: 'Reserve manpower held', value: persons(shown.entries.reduce((sum, entry) => sum + Math.max(0, entry.amount), 0)) },
      ...shown.entries.map(entry => ({ label: named(entry.source), value: persons(entry.amount), indent: true })),
    ] }];
  };
  if (!track) return null;
  if (!holder) return <WidgetState kind="empty" icon="flag" message="Choose a holder: click its bar in the race." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading this holder's reserve…" />;
  const negative = segment?.entries?.filter(entry => entry.amount < 0) ?? [];
  const list = negative.length > 0 && <dl className="occupation-negative" aria-label="Negative entries">
    <dt>Negative entries</dt>
    {negative.map((entry, index) => { const tag = sourceTagAt(data, entry.source, Math.floor(hour)); return <dd key={index}><span>{tag === null ? 'Source country unavailable' : looks.name(tag, hour)}</span><span>{persons(entry.amount)}</span></dd>; })}
  </dl>;
  if (segment?.admitted && !sources.length) return <div className="occupation-holder">
    <WidgetState kind="empty" icon="flag" message={`${looks.name(holder.tag, hour)} holds no reserve manpower at this date.`} />{list}
  </div>;
  return <div className="occupation-holder">
    <CountryRaceView runtime={runtime} track={track} active={active} options={widget.options ?? {}} view="mix" extent={extent} playhead={playhead} liveHead={liveHead}
      moment={hour} slots={1} ranked={[]} lines={[]} parts={mix?.parts ?? []} mix={mix} countryTags={[holder.tag]}
      label="Reserve manpower by source country" empty="No reserve manpower recorded." formatValue={value => Math.round(value).toLocaleString('en-US')}
      labels={{ mix: name => `Reserve manpower ${name} holds by source country` }} partsLabel="Source countries"
      presentation={{ exact: true, held: true, windowedLegend: true }}
      mixEmpty={{ icon: 'flag', message: 'Choose a holder: click its bar in the race.', unavailable: name => `Reserve manpower was not recorded for ${name} at this date.` }}
      tooltip={tooltip} />
    {list}
  </div>;
}
