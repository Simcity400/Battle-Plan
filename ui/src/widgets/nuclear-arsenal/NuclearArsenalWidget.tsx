import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import CountryPicker from '../../components/CountryPicker';
import CountryRaceView from '../../charts/race/CountryRaceView';
import DrillHost from '../../charts/drill/DrillHost';
import DrillParts from '../../charts/drill/DrillParts';
import { CLOSED, drillHour } from '../../charts/drill/drillState';
import type { DrillAction } from '../../charts/drill/drillState';
import { partColours } from '../../charts/race/countryRace';
import { UNATTRIBUTED_PART } from '../../charts/race/partsRaceDrill';
import { useChartTokens } from '../../charts/useChartTokens';
import { linesTooltip } from '../../charts/race/lines';
import { mixCountry } from '../../charts/race/options';
import type { RaceView } from '../../charts/race/options';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { countryAliasesAt } from '../../dashboard/countryIdentity';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatchFilter, isMember, projectResolvedCountries, rankedCandidates, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useHoverLabels } from '../../dashboard/useHoverLabels';
import { readAt } from '../../dashboard/reads';
import type { CountryNukesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { nuclearCountrySelection } from './NuclearCountryChip';
import { formatNukes, nuclearDrillAliasAt, nuclearDrillSelection, nuclearIdentityModel, nuclearNumbers, pinNuclearDrill, reduceNuclearDrill, validateNuclearDrillReply, nuclearDrillAt, nuclearDrillBands, nuclearModel, nuclearRuns, nuclearSegmentAt, nuclearTooltip, nukesRequest, rankNukes, validateNukesReply } from './nuclearArsenalData';

const NO_WARNINGS: string[] = [];
export default function NuclearArsenalWidget({ widget, runtime, active, countrySlot = null, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const [drillState, dispatch] = useReducer(reduceNuclearDrill, { key: null, drill: CLOSED });
  const [selection, setOrigin] = useState<{ key: string; tag: string; hour: number } | null>(null);
  const [pendingChoice, setChoice] = useState<{ key: string; hour: number; tags: readonly string[] } | null>(null);
  const anchor = useRef<HTMLDivElement>(null);
  const view = options.view as RaceView;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const resetKey = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, widget.countries, options.show, options.drillOriginHour]);
  const dispatchDrill = useCallback((action: DrillAction) => dispatch({ key: resetKey, action }), [resetKey]);
  const choice = pendingChoice?.key === resetKey ? pendingChoice : null;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const moment = Math.floor(playhead ?? extent[1]);
  const needsGroups = widget.countries.kind !== 'group' || widget.countries.group.type !== 'top';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, track?.branch.lastHour ?? 0, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const counted = useMemo(() => projectResolvedCountries(resolved, countryMatchFilter(widget.countries, fallback ? [fallback] : [],
    groups.data, track?.branch.lastHour ?? 0, undefined, track?.branch.firstHour ?? 0)),
    [resolved, widget.countries, fallback, groups.data, track?.branch.lastHour, track?.branch.firstHour]);
  const request = track && (resolved.top != null || resolved.tags.length) ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-nukes', params: nukesRequest(track.branch.id, resolved) } : null;
  const product = useProduct<CountryNukesReply>(runtime.client, request, active, validateNukesReply);
  const { error, retry } = product;
  const retained = useMemo(() => new NumericReply<CountryNukesReply>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(request), product.data, nuclearNumbers);
  const pickOwner = useMemo(() => ({ client: runtime.client, source: [track?.source.id, track?.source.revision, track?.branch.id],
    view, countries: widget.countries, membership: counted.membership, numbers }),
  [runtime.client, track?.source.id, track?.source.revision, track?.branch.id, view, widget.countries, counted.membership, numbers]);
  const data = product.data ?? numbers;
  const model = useMemo(() => data ? nuclearModel(data) : null, [data]);
  const slots = resolved.top ?? Math.max(1, counted.tags.length);
  const ranked = useMemo(() => model ? rankNukes(model, rankedCandidates(counted, moment, model.tags), moment, slots, resolved.top != null) : [],
    [model, counted, moment, slots, resolved.top]);
  const range = useMemo(() => ({ firstHour: extent[0], lastHour: extent[1] }), [extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo(() => !model ? [] : (resolved.top != null ? model.tags : counted.tags).flatMap(tag => {
    const runs = nuclearRuns(model, tag, counted.membership?.get(tag) ?? null, range);
    return runs.length ? [{ tag, runs }] : [];
  }), [model, counted, resolved.top, range]);
  const mixTag = mixCountry(widget.countries, fallback, ranked);
  const mixAlias = useMemo(() => {
    if (!mixTag || (widget.countries.kind !== 'tags' && widget.countries.kind !== 'selected')) return mixTag;
    const aliases = groups.data ? countryAliasesAt(groups.data.countryIdentities, [mixTag], moment) : [];
    return aliases.includes(mixTag) ? mixTag : aliases.length === 1 ? aliases[0] : null;
  }, [mixTag, widget.countries.kind, groups.data, moment]);
  const pinned = options.show === 'breakdown';
  const { drill, origin, tag: drillTag, hour: originHour } = nuclearDrillSelection(drillState, resetKey, selection, pinned, mixTag,
    typeof options.drillOriginHour === 'number' ? options.drillOriginHour : moment);
  usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || (drill.breakdown != null && drill.follow)));
  const followHour = track ? Math.floor(runtime.clock.getHour()) + track.axisOffset : null;
  const drillAt = Math.floor((pinned && drill.breakdown == null ? moment : drillHour(drill, followHour ?? extent[1])) ?? moment);
  useEffect(() => { if (drillTag && origin == null) setOrigin({ key: resetKey, tag: drillTag, hour: originHour }); }, [resetKey, drillTag, origin, originHour]);
  const drillRequest = track && drillTag ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'country-nukes', params: nukesRequest(track.branch.id, { tags: [drillTag] }, originHour) } : null;
  const drillReply = useProduct<CountryNukesReply>(runtime.client, drillRequest, active, validateNuclearDrillReply);
  const retainedDrill = useMemo(() => new NumericReply<CountryNukesReply>(), [runtime.client]);
  const drillNumbers = retainedDrill.update(JSON.stringify(drillRequest), drillReply.data, nuclearNumbers);
  const drillPickOwner = useMemo(() => ({ client: runtime.client, source: [track?.source.id, track?.source.revision, track?.branch.id],
    view: 'mix', tag: drillTag, originHour, numbers: drillNumbers }),
  [runtime.client, track?.source.id, track?.source.revision, track?.branch.id, drillTag, originHour, drillNumbers]);
  const drillData = drillReply.data ?? drillNumbers;
  const drillModel = useMemo(() => drillTag && drillData ? nuclearIdentityModel(drillData, drillTag) : null,
    [drillTag, drillData]);
  const aliasAt = useCallback((hour: number) => nuclearDrillAliasAt(drillData ?? null, hour), [drillData]);
  const currentAlias = aliasAt(drillAt);
  const headerSelection = nuclearCountrySelection(widget, track);
  const headerAlias = window && headerSelection?.tag === drillTag && headerSelection?.hour === originHour && !drillReply.error
    ? nuclearDrillAliasAt(drillReply.data ?? null, drillAt) : null;
  const headerWanted = useMemo(() => pinned && countrySlot && headerAlias ? [{ tag: headerAlias, hour: drillAt }] : [],
    [pinned, countrySlot, headerAlias, drillAt]);
  const headerLabels = useCountryLabelsAt(runtime.client, track, headerWanted, active && !!countrySlot && pinned);
  const header = pinned && countrySlot && createPortal(<DatedCountryChip
    label={headerAlias ? headerLabels.labelAt(drillAt)(headerAlias) : null} hour={window ? drillAt : null}
    reason={drillReply.error ? 'Couldn’t load the breakdown.' : (!headerSelection ? 'Saved country selection unavailable. Choose it again.'
      : !headerAlias ? 'Country unavailable at this time.' : null)} />, countrySlot);
  const looks = useCountryLooks(runtime.client, track, currentAlias ? [currentAlias] : [], drillAt, active,
    (tag, hour) => drillData?.series.find(series => series.tag === tag)?.segments.find(span => span.firstHour <= hour && hour <= span.lastHour)?.firstHour ?? null);
  const pendingLabel = useCallback(() => ({ name: 'Loading name…' }), []);
  const hover = useHoverLabels(runtime.client, track, active, pendingLabel);
  const entityLooks = {
    name: (_tag: string, hour = drillAt) => { const alias = aliasAt(hour); return alias ? looks.name(alias, hour) : 'Country not recorded'; },
    flag: (_tag: string, hour = drillAt) => { const alias = aliasAt(hour); return alias ? looks.flag(alias, hour) : undefined; },
    color: () => tokens.muted,
  };
  const drillParts = useMemo(() => [...(drillModel?.parts ?? []), UNATTRIBUTED_PART], [drillModel]);
  const colours = useMemo(() => partColours(drillParts, tokens.categories), [drillParts, tokens.categories]);
  const drillBands = useMemo(() => drillModel && drillTag ? nuclearDrillBands(drillModel, drillTag,
    null, range) : null, [drillModel, drillTag, range]);
  const drillValue = drillModel && drillTag
    ? nuclearDrillAt(drillModel, drillTag, drillAt) : null;
  useEffect(() => { dispatchDrill({ type: 'close' }); setChoice(null); }, [dispatchDrill]);
  const mix = useMemo(() => model && mixAlias ? { tag: mixAlias, bands: model.reply.types
    .map(type => nuclearRuns(model, mixAlias, counted.membership?.get(mixAlias) ?? null, range, type.ordinal)) } : null,
  [model, mixAlias, counted.membership, range]);
  const segmentAt = useCallback((tag: string, hour: number) => nuclearSegmentAt(model, tag, hour), [model]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(groups.data?.warnings ?? []), ...(drillReply.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, groups.data, drillReply.data, onDiagnostics]);
  if (!track) return <>{header}<WidgetState kind="empty" icon="target" message="Load a campaign to see its nuclear arsenal." /></>;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <>{header}<WidgetState kind="unavailable" message="The chosen moment is not in this campaign." /></>;
  const open = (hour: number, tag?: string, choices?: readonly string[]) => {
    // The shared chart supplies only actors drawn under this pick (#440). A
    // keyboard/overlap pick remains a choice; never choose the current leader.
    if (tag) { setChoice(null); setOrigin({ key: resetKey, tag, hour }); dispatchDrill({ type: 'open', breakdown: tag, hour,
      playing: runtime.clock.getSnapshot().playing, playhead: followHour }); }
    else if (choices?.length) setChoice({ key: resetKey, hour, tags: choices });
  };
  const breakdown = <CountryRaceView runtime={runtime} track={track} active={active} options={options} view="mix"
    extent={extent} playhead={drill.breakdown != null && drill.follow ? followHour : playhead} moment={drillAt} slots={1} ranked={[]} lines={[]} parts={drillParts}
    mix={drillTag && drillBands ? { tag: drillTag, bands: drillBands, parts: drillParts.map((part, index) => ({ ...part,
      value: part.id === '' ? drillValue?.remainder == null ? 'Not known' : formatNukes(drillValue.remainder)
        : drillValue?.values[index] == null ? 'Not known' : formatNukes(drillValue.values[index]!) })) } : null}
    label="Nuclear stockpile" empty="No nuclear stockpile was recorded at this time." formatValue={formatNukes}
    presentation={{ exact: false, held: false, countryLooks: false }} entityLooks={entityLooks} selectedHour={drillAt}
    pickOwner={drillPickOwner}
    segmentAt={(tag, hour) => nuclearSegmentAt(drillModel, tag, hour)}
    state={drillReply.error ? <WidgetState kind="error" detail={drillReply.error} onRetry={drillReply.retry} />
      : drillTag && !drillModel ? <WidgetState kind="loading" label="Loading nuke types…" /> : undefined}
    tooltip={() => null}
    timeTooltip={hour => {
      if (!drillModel || !drillTag) return null;
      const alias = aliasAt(hour);
      if (!alias) return [{ title: formatDateTime(hour), rows: [{ label: 'Country', value: 'Not recorded' }] }];
      hover.ask(hour, [alias]);
      const forms = nuclearTooltip(drillModel, drillTag, hour, hover.labelAt(hour)(alias).name, colours, formatDateTime(hour));
      const at = nuclearDrillAt(drillModel, drillTag, hour);
      return forms?.map(form => ({ ...form, rows: [...form.rows.slice(0, -1), ...(at?.remainder ? [{
        label: UNATTRIBUTED_PART.label, value: formatNukes(at.remainder), swatch: colours.get(''),
      }] : []), ...form.rows.slice(-1)] })) ?? null;
    }}
    beforeChart={<div className="parts-race-details">
      {pinned && <p className="drill-empty">{formatDateTime(drillAt)} {followHour != null && (drill.breakdown == null ? widget.time.mode !== 'follow' : !drill.follow)
        && <button type="button" className="button small" onClick={() => {
          const hour = followHour;
          if (hour != null && drillTag) dispatchDrill({ type: 'open', breakdown: drillTag, hour, playing: true, playhead: hour });
        }}>Follow playback</button>}</p>}
      <DrillParts parts={(drillModel?.parts ?? []).map((part, index) => ({ ...part,
        value: drillValue?.values[index] ?? null, text: drillValue?.values[index] == null ? 'Not recorded' : formatNukes(drillValue.values[index]!), color: colours.get(part.id), muted: drillValue?.values[index] == null || drillModel?.reply.types[index].name == null }))}
        format={formatNukes} total={drillValue?.total ?? null}
        unattributed={drillValue?.remainder == null ? undefined : { ...UNATTRIBUTED_PART, value: drillValue.remainder, raw: BigInt(drillValue.remainder), color: colours.get('') }}
        totalRow={{ label: 'Nuclear stockpile', value: drillValue?.total == null ? 'Not recorded' : formatNukes(drillValue.total) }} />
    </div>}
    onPick={drillTag ? hour => dispatchDrill(pinned ? { type: 'open', breakdown: drillTag, hour, playing: false, playhead: null }
      : { type: 'pick', hour }) : undefined} />;
  if (pinned) return <>{header}<div className="parts-race-drill">{breakdown}</div></>;
  const overviewState = error ? <WidgetState kind="error" detail={error} onRetry={retry} />
    : groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />
      : resolved.reason ? <WidgetState kind="empty" icon="target" message={resolved.reason} />
        : !model || !data || (needsGroups && !groups.data) ? <WidgetState kind="loading" label="Loading nuclear arsenal…" />
          : !model.tags.length && resolved.top == null ? <WidgetState kind="unavailable" message="No nuclear stockpiles were recorded for these countries." /> : undefined;
  const main = <CountryRaceView runtime={runtime} track={track} active={active && (drill.breakdown == null || options.drill === 'popout')}
    options={options} view={view} extent={extent} playhead={playhead} onPick={open} pickOwner={pickOwner}
    moment={moment} slots={slots} ranked={ranked} lines={lines} parts={model?.parts} mix={mix} segmentAt={segmentAt} state={overviewState}
    presentation={{ exact: false, held: false }}
    label="Nuclear stockpile" empty={resolved.top != null && readAt(data?.emptyReadHours ?? [], moment) === moment
      ? 'No nuclear weapons yet.' : 'No nuclear stockpile was recorded at this time.'} formatValue={formatNukes}
    tooltip={(tag, hour, name, colours) => {
      if (!model) return null;
      const membership = model.series.get(tag)?.membership;
      return isMember(membership ?? undefined, hour) && isMember(counted.membership?.get(tag), hour)
        ? nuclearTooltip(model, tag, hour, name, colours, formatDateTime(hour)) : null;
    }} lineTooltip={(hour, tags, labelAt, country) => [{
      ...linesTooltip(hour, lines.filter(line => tags.includes(line.tag)), labelAt, country, formatDateTime(hour), 'Nuclear stockpile', formatNukes),
    }]} beforeChart={choice && <div className="drill-empty" role="group" aria-label="Choose the nuclear arsenal to open">
      <p>{formatDateTime(choice.hour)} · Choose a country</p>
      <CountryPicker countries={track.source.metadata.countries} selectedTag={null} playerCountryId={null}
        source={track.source} client={runtime.client} field allowedTags={choice.tags} flagAt={{ branchId: track.branch.id, hour: choice.hour }}
        addLabel="Choose a country" onChange={tag => {
          if (tag) { setOrigin({ key: resetKey, tag, hour: choice.hour }); dispatchDrill({ type: 'open', breakdown: tag, hour: choice.hour, playing: false, playhead: null }); setChoice(null); }
        }} />
      <button type="button" className="button small quiet" onClick={() => setChoice(null)}>Cancel</button>
    </div>} />;
  return <div className="parts-race" ref={anchor}><DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'}
    open={drill.breakdown != null} anchor={anchor} title="Weapon types" day={formatDateTime(drillAt)} following={drill.follow}
    canFollow={followHour != null} onClose={() => dispatchDrill({ type: 'close' })} onFollow={() => dispatchDrill({ type: 'follow' })}
    onPin={drillTag ? () => runtime.addWidgetBelow(widget.id, pinNuclearDrill(widget, track, drillTag, drillAt, originHour, drill.follow)) : undefined}
    main={main}>{breakdown}</DrillHost></div>;
}
