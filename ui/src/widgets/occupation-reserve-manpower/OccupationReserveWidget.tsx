import { asFailure } from '../../dashboard/queryError.ts';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import * as requests from '../../dashboard/batch5Requests';
import WidgetState from '../../components/WidgetState';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { CountryRaceEntry, CountryRacePart } from '../../charts/race/countryRace';
import { useCountryColours } from '../../charts/race/countryColours';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { PreparedKey } from '../../charts/PreparedKey';
import { useChartTokens } from '../../charts/useChartTokens';
import type { TooltipContent } from '../../charts/tooltip';
import { countryPart } from '../../dashboard/keyedHistory';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { lookHour, useCountryLooks } from '../../dashboard/countryLooks';
import { useHolderLooks } from '../../dashboard/holderLooks';
import type { HolderAt } from '../../dashboard/holderLooks';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING, countryMatchFilter, noMembersAt, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, setToken } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { pendingReplyAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useProduct } from '../../dashboard/products';
import { BREAKDOWN } from './definition';
import { holderKeyAt, holderSourceBands, persons, reservesAt, sourceParts, sourceWitnesses, validateOccupationReserve } from './occupationReserveData';
import type { HolderKeys } from './occupationReserveData';
import type { HolderReserve, OccupationReserveReply } from './occupationReserveData';
import { useOccupationReserve } from './useOccupationReserve';
import './occupationReserve.css';

const UNRESOLVED = 'unresolved';

/** Foreign garrison manpower each country holds in reserve, as a ranked race whose bars
 * split by the country that sent it; a holder opens its sources over time. */
export default function OccupationReserveWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics } = props;
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  // A drill that follows playback follows it whatever the widget's own time.
  const playbackHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || drill.breakdown !== null && drill.follow));
  const tokens = useChartTokens();
  const anchor = useRef<HTMLDivElement>(null);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const all = widget.countries.kind === 'all';
  const grouped = widget.countries.kind === 'group' && widget.countries.group.type !== 'top';
  const manual = widget.countries.kind === 'tags' || widget.countries.kind === 'selected';
  const groups = useCountryGroups(runtime, track, active && (grouped || manual || drill.breakdown !== null || widget.options?.show === 'breakdown'));
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const identities = useMemo(() => groups.data ? countryIdentityIndex(groups.data.countryIdentities) : null, [groups.data]);
  // Chosen countries and group members alike follow their native holders through renames.
  const counted = useMemo(() => (manual || grouped) && identities ? projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, identities, track?.branch.firstHour ?? 0)) : resolved,
  [manual, grouped, identities, resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour]);
  // Chosen tags are asked for under every tag their holders held; by their literal tags if the history fails.
  const asked = widget.countries.kind !== 'tags' ? undefined : identities ? counted.tags : groups.error ? widget.countries.tags : null;
  const { options, data, error, retry } = useOccupationReserve(widget, runtime, active, asked);
  const at = useMemo(() => data ? reservesAt(data, moment) : null, [data, moment]);
  const readHour = at?.readHour ?? moment;
  const shown = useMemo(() => {
    if (!at) return [];
    // Admitted holders and any with a recorded entry stay reachable, proven zeros included.
    const ranked = [...at.holders].filter(holder => holder.admitted || holder.entries.length > 0)
      .sort((a, b) => a.positive > b.positive ? -1 : a.positive < b.positive ? 1 : a.tag.localeCompare(b.tag));
    if (all) return ranked;
    const candidates = new Set(rankedCandidates(counted, moment, ranked.map(holder => holder.tag)));
    const kept = ranked.filter(holder => candidates.has(holder.tag));
    return resolved.top != null ? kept.slice(0, resolved.top) : kept;
  }, [at, all, counted, resolved.top, moment]);
  const sources = useMemo(() => [...new Set(shown.flatMap(holder => holder.entries.map(entry => entry.source ?? UNRESOLVED)))], [shown]);
  const sourceTags = useMemo(() => sources.flatMap(source => source === UNRESOLVED ? [] : [countryPart(source)!.tag]), [sources]);
  const holderTags = useMemo(() => shown.map(holder => holder.tag), [shown]);
  // Holders and sources are named at the reserve read that proves them, never a sampled earlier hour.
  const lookTags = useMemo(() => [...new Set([...holderTags, ...sourceTags])], [holderTags, sourceTags]);
  const lookHours = useMemo(() => new Map(lookTags.map(tag => [tag, readHour])), [lookTags, readHour]);
  const looks = useCountryLooks(runtime.client, track, lookTags, readHour, active, undefined, lookHours, true);
  const colour = useCountryColours(sourceTags, looks.color, tokens, looks);
  const sourceColour = (source: string | null) => source === null || source === UNRESOLVED ? tokens.muted : colour(countryPart(source)!.tag, readHour);
  const sourceName = (source: string | null) => source === null || source === UNRESOLVED ? 'Source country unavailable' : looks.name(countryPart(source)!.tag, readHour);
  const parts = useMemo<CountryRacePart[]>(() => sources.map(source => ({ id: source, label: sourceName(source), color: sourceColour(source) })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sources, looks, colour]);
  const entries = useMemo<CountryRaceEntry[]>(() => shown.map(holder => ({ tag: holder.tag, value: Number(holder.positive), label: persons(holder.positive), stretch: null,
    parts: sourceParts(holder).map(part => ({ id: part.source ?? UNRESOLVED, value: Number(part.amount), color: sourceColour(part.source) })) })),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [shown, looks, colour]);
  const byTag = useMemo(() => new Map(shown.map(holder => [holder.tag, holder])), [shown]);
  const [holder, setHolder] = useState<string | null>(null);
  const pinned = options.show === 'breakdown';
  const pinnedHolder = pinned && options.holder ? contentKey(String(options.holder)) : null;
  const drilled = pinned ? pinnedHolder : drill.breakdown ? holder : null;
  // Playback in this campaign's hours, within its branch.
  const clockHour = track ? Math.max(track.branch.firstHour, Math.min(Math.floor(playbackHour) + track.axisOffset, track.branch.lastHour)) : moment;
  const drillAt = pinned ? moment : drillHour(drill, clockHour);
  const drilledPart = drilled ? countryPart(drilled) : null;
  const drilledAliases = identities && drilledPart ? identities.holderAliases(drilledPart.tag, drilledPart.slot) : null;
  const drilledTag = drilledAliases ? drilledAliases.find(run => drillAt !== null && run.firstHour <= drillAt && drillAt <= run.lastHour)?.tag ?? null : drilledPart?.tag ?? null;
  const drilledNames = useCountryLabelsAt(runtime.client, track, drilledTag && drillAt !== null ? [{ tag: drilledTag, hour: drillAt }] : [], active);
  const none = grouped ? noMembersAt(widget.countries, resolved, groups.data, moment, track?.branch.lastHour ?? 0) : null;
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...groups.data?.warnings ?? []] : null);
  if (!track) return <WidgetState kind="empty" icon="flag" message="Load a campaign to see occupation manpower." />;
  if (!window && widget.time.mode !== 'campaign') return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (pinned) return <HolderSources {...props} holder={pinnedHolder} hour={moment} extent={extent} playhead={playhead} />;
  if (error) return <WidgetState kind="error" failure={asFailure(error)} onRetry={retry} />;
  const groupFailure = grouped && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (resolved.reason && widget.countries.kind !== 'all') return <>{groupFailure}<WidgetState kind="empty" message={resolved.reason} /></>;
  // A group with no member found at the moment says why once that moment is read, and loads until then.
  if (none) return <>{groupFailure}<NoMembersState reason={none.reason} icon="flag" /></>;
  if (!data || grouped && !groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading occupation manpower…" /></>;
  if (pendingReplyAt(data, moment)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  if (!at) return <>{groupFailure}<WidgetState kind="unavailable" message="Occupation manpower was not recorded at this time." /></>;
  const tooltip = (tag: string, _hour: number, name: string): TooltipContent[] | null => {
    const reserve = byTag.get(tag);
    return reserve ? [holderTooltip(reserve, name, sourceName, sourceColour, dayLabel(readHour))] : null;
  };
  const main = <CountryRaceView runtime={runtime} track={track} active={active && !drill.breakdown} options={options} view="race" extent={extent}
    playhead={playhead} moment={moment} slots={resolved.top ?? Math.max(1, shown.length)} ranked={entries} lines={[]} parts={parts}
    raceLegendParts={parts} partsLabel="Source countries" countryTags={holderTags} countryLookHours={lookHours} label="Occupation reserve manpower"
    // A group's members at an hour still being prepared are not known yet: no bar is not proof of none.
    empty={resolved.known(moment) ? 'No reserve manpower is held at this date.' : MEMBERSHIP_PREPARING} formatValue={value => Math.round(value).toLocaleString('en-US')}
    labels={{ race: `Occupation reserve manpower, ${dayLabel(moment)}`, actions: 'Occupation reserve actions' }}
    presentation={{ exact: true, windowedLegend: true }} tooltip={tooltip}
    onPick={(hour, tag) => { if (!tag) return; setHolder(shown.find(item => item.tag === tag)?.holder ?? null); dispatch({ type: 'open', breakdown: BREAKDOWN.id, hour, playing: widget.time.mode === 'follow', playhead: widget.time.mode === 'follow' ? moment : null }); }} />;
  const prefix = preparedPrefix(data);
  return <div ref={anchor} className="parts-race occupation-reserve">{groupFailure}
    <PreparedKey pending={prefix?.pending} approximate={prefix?.approximate} />
    <DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drilled !== null}
      title={drilledTag && drillAt !== null ? drilledNames.labelAt(drillAt)(drilledTag).name : BREAKDOWN.label} day={drillAt === null ? '' : dayLabel(drillAt)}
      following={drill.follow} canFollow onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}
      onPin={drilled ? () => {
        const pin = pinnedBreakdown(widget, BREAKDOWN, track.id);
        pin.options = { ...pin.options, holder: setToken(drilled) };
        runtime.addWidgetBelow(widget.id, pin);
      } : undefined}>
      {drilled && <HolderSources {...props} holder={drilled} hour={drillAt ?? moment} extent={extent} playhead={drill.follow ? clockHour : null} />}
    </DrillHost>
  </div>;
}

function holderTooltip(reserve: HolderReserve, name: string, sourceName: (source: string | null) => string, sourceColour: (source: string | null) => string, read: string): TooltipContent {
  const positive = reserve.entries.filter(entry => entry.amount >= 0n);
  return {
    title: name, scrollable: true, scrollId: reserve.holder,
    rows: [
      { label: 'Reserve manpower held', value: persons(reserve.positive) },
      ...positive.map(entry => ({ label: sourceName(entry.source), swatch: sourceColour(entry.source), value: persons(entry.amount), indent: true })),
      ...reserve.negative.length ? [{ label: 'Negative entries' }, ...reserve.negative.map(entry => ({ label: sourceName(entry.source), value: persons(entry.amount), indent: true }))] : [],
      { label: 'As of', value: read },
    ],
  };
}

/** One holder's reserve from each source over time, stacked by source. */
function HolderSources({ widget, runtime, active, holder: chosen, hour, extent, playhead }: WidgetProps & { holder: string | null; hour: number; extent: [number, number]; playhead: number | null }) {
  const tokens = useChartTokens();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  // The holder follows its native country through renames: every TAG@slot it was is asked for,
  // and the one it held at the hour shown is drawn.
  const groups = useCountryGroups(runtime, track, active && chosen !== null);
  const chosenPart = chosen ? countryPart(chosen) : null;
  const aliases = useMemo(() => groups.data && chosenPart ? countryIdentityIndex(groups.data.countryIdentities).holderAliases(chosenPart.tag, chosenPart.slot) : null,
    [groups.data, chosenPart?.tag, chosenPart?.slot]); // eslint-disable-line react-hooks/exhaustive-deps
  const keys = useMemo(() => aliases && chosenPart ? [...new Set(aliases.map(run => `${run.tag}@${chosenPart.slot}`))] : chosen ? [chosen] : [],
    [aliases, chosen]); // eslint-disable-line react-hooks/exhaustive-deps
  const current = aliases?.find(run => run.firstHour <= hour && hour <= run.lastHour);
  const holder = current && chosenPart ? `${current.tag}@${chosenPart.slot}` : chosen;
  // Each alias over its own hours: the drill keeps the native country's whole history.
  const aliasSpans = useMemo(() => aliases && chosenPart ? aliases.map(run => ({ key: `${run.tag}@${chosenPart.slot}`, firstHour: run.firstHour, lastHour: run.lastHour })) : null,
    [aliases, chosenPart?.slot]); // eslint-disable-line react-hooks/exhaustive-deps
  const holderKeys: HolderKeys | null = aliasSpans ?? holder;
  const tagAt = useCallback((at: number) => { const key = holderKeys === null ? null : holderKeyAt(holderKeys, at); return key ? countryPart(key)?.tag ?? null : null; }, [holderKeys]);
  const settled = !!groups.data || !!groups.error;
  const request = track && chosen && settled ? { sourceId: track.source.id, revision: track.source.revision, product: 'occupation-reserve-manpower',
    params: keys.length > 1 ? requests.occupationRace(track.branch.id, keys) : requests.occupationHolder(track.branch.id, keys[0]) } : null;
  const { data, error, retry } = useProduct<OccupationReserveReply>(runtime.client, request, active, validateOccupationReserve, track?.source);
  const at = useMemo(() => data ? reservesAt(data, hour)?.holders.find(item => item.holder === holder) ?? null : null, [data, hour, holder]);
  // Resolved sources, then one band for amounts whose source does not resolve.
  const drill = useMemo(() => data && holder && holderKeys ? holderSourceBands(data, holderKeys) : { sources: [] as (string | null)[], bands: [] }, [data, holder, holderKeys]);
  const sources = drill.sources;
  const holderTag = holder ? countryPart(holder)?.tag ?? holder : null;
  // Named at the reserve read that proves the holder, never a sampled earlier hour.
  const readAt = data ? reservesAt(data, hour)?.readHour ?? Math.floor(hour) : Math.floor(hour);
  const holderHours = useMemo(() => new Map(holderTag ? [[holderTag, readAt]] : []), [holderTag, readAt]);
  const looks = useCountryLooks(runtime.client, track, useMemo(() => holderTag ? [holderTag] : [], [holderTag]), readAt, active, undefined, holderHours, true);
  // One colour per source over the whole history, as history charts colour a series: read where
  // the tag is proven the source's own, so a later holder of the tag never lends its look.
  const sample = track ? lookHour(track.branch, hour)?.hour ?? Math.floor(hour) : Math.floor(hour);
  const witnessKey = JSON.stringify(data && holder && holderKeys ? sourceWitnesses(data, holderKeys, hour, sample) : []);
  const witnesses = useMemo(() => JSON.parse(witnessKey) as HolderAt[], [witnessKey]);
  const sourceLooks = useHolderLooks(runtime.client, track, witnesses, active, tokens);
  const sourceName = (source: string | null) => source === null ? 'Source country unavailable' : sourceLooks.name(source);
  const sourceColour = (source: string | null) => source === null ? tokens.muted : sourceLooks.colour(source);
  const bands = drill.bands;
  const mix = useMemo(() => holderTag ? { tag: holderTag, tagAt: aliasSpans ? tagAt : undefined, bands, parts: sources.map(source => {
    const amount = at ? sourceParts(at).find(part => part.source === source)?.amount ?? (at.admitted ? 0n : undefined) : undefined;
    return { id: source ?? UNRESOLVED, label: sourceName(source), color: sourceColour(source), value: amount === undefined ? '—' : persons(amount) };
  }) } : null,
  // eslint-disable-next-line react-hooks/exhaustive-deps
  [holderTag, bands, sources, at, sourceLooks, aliasSpans, tagAt]);
  // A history tooltip names its sources as they were at the read it shows; swatches are the bands' colours.
  const [reached, setReached] = useState<number | null>(null);
  const datedWanted = useMemo(() => {
    const reads = [{ reserve: at, hour: readAt }];
    if (data && reached !== null && holderKeys !== null) {
      const read = reservesAt(data, reached), key = holderKeyAt(holderKeys, reached);
      reads.push({ reserve: read?.holders.find(item => item.holder === key) ?? null, hour: read?.readHour ?? Math.floor(reached) });
    }
    return reads.flatMap(({ reserve, hour }) => [...new Set(reserve?.entries.flatMap(entry => entry.source === null ? [] : [countryPart(entry.source)!.tag]))]
      .map(tag => ({ tag, hour })));
  }, [at, readAt, data, reached, holderKeys]);
  const dated = useCountryLabelsAt(runtime.client, track, datedWanted, active);
  const tooltip = (_tag: string, hourAt: number, name: string): TooltipContent[] | null => {
    const read = data && holder ? reservesAt(data, hourAt) : null;
    const key = holderKeys === null ? null : holderKeyAt(holderKeys, hourAt);
    const reserve = read?.holders.find(item => item.holder === key);
    if (!read || !reserve) return null;
    const readHour = read.readHour ?? Math.floor(hourAt);
    const datedName = (source: string | null) => source === null ? 'Source country unavailable' : dated.labelAt(readHour)(countryPart(source)!.tag).name;
    return [holderTooltip(reserve, name, datedName, sourceColour, dayLabel(readHour))];
  };
  if (!track) return null;
  if (!holder) return <WidgetState kind="empty" icon="flag" message="Choose a holder: click its bar in the race." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading this holder's reserve…" />;
  if (pendingReplyAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  const negative = at?.negative ?? [];
  const list = negative.length > 0 && <dl className="occupation-negative" aria-label="Negative entries">
    <dt>Negative entries</dt>
    {negative.map(entry => <dd key={entry.ordinal}><span>{sourceName(entry.source)}</span><span>{persons(entry.amount)}</span></dd>)}
  </dl>;
  // Admitted with no positive entry anywhere in the range: nothing to stack, a proven zero.
  if (at?.admitted && !sources.length) return <div className="occupation-holder">
    <WidgetState kind="empty" icon="flag" message={`${looks.name(holderTag!, readAt)} holds no reserve manpower at this date.`} />{list}
  </div>;
  return <div className="occupation-holder">
    <CountryRaceView runtime={runtime} track={track} active={active} options={widget.options ?? {}} view="mix" extent={extent} playhead={playhead}
    moment={hour} slots={1} ranked={[]} lines={[]} parts={mix?.parts ?? []} mix={mix} pending={preparedPrefix(data)?.pending}
    countryTags={aliasSpans ? [...new Set(aliasSpans.map(run => countryPart(run.key)!.tag))] : holderTag ? [holderTag] : []}
    // Named at the reserve read that proves the alias held then, never a sampled earlier hour.
    countryLookHour={(tag, at) => tag !== null && tag === tagAt(at) ? reservesAt(data, at)?.readHour ?? Math.floor(at) : null}
    label="Reserve manpower by source country" empty="No reserve manpower recorded." formatValue={value => Math.round(value).toLocaleString('en-US')}
    labels={{ mix: name => `Reserve manpower ${name} holds by source country` }} partsLabel="Source countries"
    presentation={{ exact: true, held: true, windowedLegend: true }}
    mixEmpty={{ icon: 'flag', message: 'Choose a holder: click its bar in the race.', unavailable: name => `Reserve manpower was not recorded for ${name} at this date.` }}
    tooltip={tooltip} onReachHour={setReached} />
    {list}
  </div>;
}

