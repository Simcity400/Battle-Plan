import { failureText } from '../../dashboard/queryError.ts';
import { useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { Segmented } from '../../components/Segmented';
import CountryPicker from '../../components/CountryPicker';
import { useContextMenu } from '../../components/ContextMenu';
import { dayLabel } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import { SECONDARY_DASH } from '../../charts/palette';
import { jumpItem } from '../../charts/jump';
import CountryRaceView from '../../charts/race/CountryRaceView';
import type { CountryRaceLine } from '../../charts/race/countryRace';
import { clipToSpans, runAt } from '../../charts/series';
import { barStrip } from '../../charts/strip';
import type { ChartStrip } from '../../charts/TimeChart';
import { CHART_LAYOUT } from '../../charts/tokens';
import { useChartTokens } from '../../charts/useChartTokens';
import type { PlaybackMode } from '../../charts/view';
import { useComparisonCountries } from '../../dashboard/comparisonCountries';
import { countryDrillTagAt } from '../../dashboard/countryIdentity';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProducts } from '../../dashboard/products';
import { NumericReply } from '../../dashboard/numericReply';
import { groupNumbers } from '../../dashboard/groupsReply';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { CountryGroupReply, FuelReply } from '../../generated/dashboard';
import { BREAKDOWNS, OPTIONS } from './definition';
import FuelBreakdown from './FuelBreakdown';
import FuelConsumptionBreakdown from './FuelConsumptionBreakdown';
import { consumptionForActor, fuelConsumptionLevelToken, fuelConsumptionToken, readFuelConsumption, readFuelConsumptionLevel } from './fuelConsumption';
import type { FuelConsumptionLevel, FuelConsumptionSelection } from './fuelConsumption';
import { fuelExtentToken, fuelMoment, fuelOpen, fuelRecipientAtPick, fuelRecipientForReply, fuelRecipientToken, readFuelExtent, readFuelRecipient } from './fuelDrill';
import type { FuelRecipientSelection } from './fuelDrill';
import { usePreparedFuel, scaledFuelSummary, fuelSummaryStrip, useFuelSeriesParity } from './preparedFuel';
import { fuelCountrySpans, fuelCountryToken, readFuelCountry } from './fuelCountry';
import type { FuelCountrySelection } from './fuelCountry';
import { formatFuel, fuelChoiceKey, fuelDecimals, fuelNumbers, fuelRuns, fuelTooltips, fuelView, validateFuelReply } from './fuelData';
import { motionOf } from '../../charts/easing.ts';

const STRIP_SHARE = 0.28;

export default function FuelWidget({ widget, runtime, active, countrySlot, onDiagnostics }: WidgetProps) {
  const [storedDrill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [drillOwner, setDrillOwner] = useState<object>();
  const clockHour = usePlaybackHour(runtime.clock, active && (widget.time.mode === 'follow' || storedDrill.breakdown !== null && storedDrill.follow));
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const digits = fuelDecimals(options);
  const { track, countries, pending, contentPending: groupContentPending, error: groupError, refreshError: groupRefreshError, retry: retryGroups } = useComparisonCountries(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const playhead = window && (widget.time.mode === 'follow' || widget.time.mode === 'pin') ? window.lastHour : null;
  const heldExtent = track && options.show !== 'main' ? readFuelExtent(options.fuelExtent, track.source.metadata.campaignKey, track.branch.id) : null;
  const extent: [number, number] = heldExtent ?? (track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour]
    : [track.branch.firstHour, track.branch.lastHour]) : [0, 1]);
  const requests = track ? countries.tags.map(tag => ({ sourceId: track.source.id, revision: track.source.revision, product: 'country-fuel',
    params: { branchId: track.branch.id, tag } })) : [];
  const prepared = usePreparedFuel(runtime.client, track, countries.tags, Math.floor(playhead ?? extent[1]), active, ['fuel','capacity','stripGain','stripBurn'], countries.membership ?? undefined);
  const legacyProduct = useProducts<FuelReply>(runtime.client, requests, active && !prepared.enabled, validateFuelReply);
  const product = prepared.enabled ? prepared : legacyProduct;
  const { retry } = product;
  const error = useHeldFailure(product.error, product.data !== undefined, retry);
  const retained = useMemo(() => new NumericReply<FuelReply[]>(), [runtime.client]);
  const numbers = retained.update(JSON.stringify(requests), product.data, fuelNumbers);
  const data = product.data ?? numbers;
  const contentPending = !!numbers && !product.data && !error;
  const single = data?.length === 1;
  const [chosenTag, setChosenTag] = useState<string>();
  const [pickTags, setPickTags] = useState<readonly string[]>();
  const [measure, setMeasure] = useState<'gain' | 'burn' | 'consumption'>('consumption');
  const anchor = useRef<HTMLDivElement>(null);
  const saved = BREAKDOWNS.find(item => item.id === options.show);
  const menu = useContextMenu('Fuel actions');
  const candidates = useMemo(() => fuelChoiceKey(widget.countries, countries, options.capacity !== false),
    [widget.countries, countries.tags, countries.membership, options.capacity]); // eslint-disable-line react-hooks/exhaustive-deps
  const pickScope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, candidates, options.show,
    extent, options.playback, options.afterPlayhead, options.windowDays, options.log, options.yMin, options.yMax]);
  const pickOwner = useMemo(() => ({ scope: pickScope, numbers: prepared.enabled ? prepared.owner : numbers, client: runtime.client, active }), [pickScope, prepared.enabled ? prepared.owner : numbers, runtime.client, active]);
  const drill = drillOwner === undefined || drillOwner === pickOwner ? storedDrill : CLOSED;
  const literalChosen = data?.find(reply => reply.tag === chosenTag) ?? (pickTags ? undefined : data?.[0]);
  const drillMeasure = saved?.id ?? BREAKDOWNS.find(item => item.id === drill.breakdown)?.id ?? measure;
  const actualHour = track ? clockHour + track.axisOffset : null;
  const at = fuelMoment(drill, !!saved, playhead ?? extent[1], actualHour);
  const savedRecipient = track && saved?.id === 'burn' ? readFuelRecipient(options.fuelRecipient, track.source.metadata.campaignKey, track.branch.id) : null;
  const [recipient, setRecipient] = useState<FuelRecipientSelection | null | undefined>(undefined);
  const retainedRecipient = recipient === undefined ? savedRecipient : recipient;
  const savedConsumption = track && saved?.id === 'consumption' ? readFuelConsumption(options.fuelConsumption, track.source.metadata.campaignKey, track.branch.id) : null;
  const [consumption, setConsumption] = useState<FuelConsumptionSelection | null | undefined>(undefined);
  const retainedConsumption = consumption === undefined ? savedConsumption : consumption;
  const savedLevel = track && saved?.id === 'consumption' ? readFuelConsumptionLevel(options.fuelConsumptionLevel, track.source.metadata.campaignKey, track.branch.id) : null;
  const [level, setLevel] = useState<FuelConsumptionLevel | null | undefined>(undefined);
  const selectedLevel = level === undefined ? savedLevel : level;
  const savedCountry = useMemo(() => track && saved ? readFuelCountry(options.fuelCountry, track.source.metadata.campaignKey, track.branch.id, widget.countries) : null,
    [!!saved, options.fuelCountry, widget.countries, track?.source.metadata.campaignKey, track?.branch.id]); // eslint-disable-line react-hooks/exhaustive-deps
  const [countryPick, setCountryPick] = useState<{ owner: object; selection: FuelCountrySelection } | null>(null);
  const selectedCountry = saved ? savedCountry : countryPick?.owner === pickOwner ? countryPick.selection : null;
  const catalogue = useCountryGroups(runtime, track, active && !!selectedCountry);
  const catalogueStore = useMemo(() => new NumericReply<CountryGroupReply>(), [runtime.client]);
  const identityNumbers = catalogueStore.update(JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id]), catalogue.data, groupNumbers);
  const actorSpans = useMemo(() => selectedCountry && identityNumbers && track && identityNumbers.branchId === track.branch.id ? fuelCountrySpans(identityNumbers.countryIdentities,
    selectedCountry, track.branch.firstHour, track.branch.lastHour, countries.membership) : null,
  [selectedCountry, identityNumbers, track?.branch.firstHour, track?.branch.lastHour, countries.membership]);
  const identitySpans = useMemo(() => selectedCountry && identityNumbers && track ? fuelCountrySpans(identityNumbers.countryIdentities,
    selectedCountry, track.branch.firstHour, track.branch.lastHour) : null,
  [selectedCountry, identityNumbers, track?.branch.firstHour, track?.branch.lastHour]);
  const actorRequests = track && actorSpans ? [...actorSpans.keys()].map(tag => ({ sourceId: track.source.id, revision: track.source.revision,
    product: 'country-fuel', params: { branchId: track.branch.id, tag } })) : [];
  const actorPrepared = usePreparedFuel(runtime.client, track, [...(actorSpans?.keys() ?? [])], Math.floor(at ?? extent[1]), active && !!selectedCountry, [], actorSpans ?? undefined);
  const actorLegacyProduct = useProducts<FuelReply>(runtime.client, actorRequests, active && !!selectedCountry && !actorPrepared.enabled, value => {
    const result = validateFuelReply(value);
    if (!track || result.branchId !== track.branch.id || !actorSpans?.has(result.tag)) throw new Error('The selected fuel history belongs to another country or branch.');
    return result;
  });
  const actorProduct = actorPrepared.enabled ? actorPrepared : actorLegacyProduct;
  const actorError = useHeldFailure(actorProduct.error, actorProduct.data !== undefined, actorProduct.retry);
  const actorStore = useMemo(() => new NumericReply<FuelReply[]>(), [runtime.client]);
  const actorNumbers = actorStore.update(JSON.stringify(actorRequests), actorProduct.data, fuelNumbers);
  const actorData = actorProduct.data ?? actorNumbers;
  const shownTag = actorSpans && at !== null ? countryDrillTagAt(actorSpans, at) : null;
  // The displayed body follows the actor; detail requests retain their original supplying basis.
  const chosen = selectedCountry ? actorData?.find(reply => reply.tag === shownTag)
    ?? actorData?.find(reply => reply.tag === selectedCountry.tag) ?? actorData?.find(reply => actorSpans?.has(reply.tag)) : literalChosen;
  const detailHour = selectedCountry && chosen?.tag !== shownTag ? null : at;
  const selectedRecipient = drillMeasure === 'burn' && chosen && track ? fuelRecipientForReply(retainedRecipient, chosen, track.source.metadata.campaignKey, identitySpans ?? undefined) : null;
  const selectedConsumption = drillMeasure === 'consumption' && chosen && track ? consumptionForActor(retainedConsumption, track.source.metadata.campaignKey, track.branch.id, chosen.tag, identitySpans ?? undefined) : null;
  const actorWithin = useMemo(() => actorSpans ? [...actorSpans.values()].flat().sort((a, b) => a.firstHour - b.firstHour) : undefined, [actorSpans]);
  const detailPickOwner = useMemo(() => ({ pickOwner, tag: chosen?.tag, measure: drillMeasure,
    numbers: selectedCountry ? actorPrepared.enabled ? actorPrepared.owner : actorNumbers : prepared.enabled ? prepared.owner : numbers, identities: selectedCountry ? identityNumbers : undefined }),
  [pickOwner, chosen?.tag, drillMeasure, selectedCountry, actorPrepared.enabled ? actorPrepared.owner : actorNumbers, prepared.enabled ? prepared.owner : numbers, identityNumbers]);
  const pickRef = useRef({ main: pickOwner, detail: detailPickOwner, active });
  pickRef.current = { main: pickOwner, detail: detailPickOwner, active };
  useEffect(() => {
    pickRef.current.active = active;
    return () => { pickRef.current.active = false; };
  }, [active, pickOwner]);
  const actorContentPending = !!selectedCountry && !!actorNumbers && !actorProduct.data && !actorProduct.error;
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(undefined); setCountryPick(null); setChosenTag(undefined); setPickTags(undefined); setRecipient(undefined); setConsumption(undefined); setLevel(undefined); }, [pickOwner]);
  const diagnostics = useMemo(() => [...(groupContentPending ? ['Group names are updating from the recorded content.'] : []),
    ...(contentPending || actorContentPending ? ['Fuel source and consumer names are updating from the recorded content.']
      : (selectedCountry ? actorData : data)?.flatMap(reply => reply.warnings.map(note => `${reply.tag}: ${note}`)) ?? [])],
  [data, actorData, selectedCountry, contentPending, actorContentPending, groupContentPending]);
  useDiagnostics(onDiagnostics, diagnostics);
  const views = useMemo(() => (data ?? []).map(reply => ({ reply, view: fuelView(reply, extent[0], extent[1]) })),
    [data, extent[0], extent[1]]); // eslint-disable-line react-hooks/exhaustive-deps
  const lines = useMemo<CountryRaceLine[]>(() => views.flatMap(({ reply, view }) => {
    const line = (of: 'fuel' | 'capacity'): CountryRaceLine => ({ tag: reply.tag, id: `${reply.tag}:${of}`,
      runs: clipToSpans(fuelRuns(view.reads, reply.quantityScale, of), countries.membership?.get(reply.tag)
        ?? [{ firstHour: extent[0], lastHour: extent[1] }]),
      ...(single ? { color: of === 'fuel' ? tokens.accent : tokens.muted } : {}),
      ...(of === 'capacity' ? { dash: SECONDARY_DASH, endDot: false, muted: true } : { area: single && options.areaFill !== false }),
      endLabel: (hour, name) => {
        const read = runAt(view.reads, hour);
        if (!read) return single ? (of === 'fuel' ? 'Fuel' : 'Capacity') : name;
        return { text: `${single ? '' : `${name} · `}${of === 'fuel' ? 'Fuel' : 'Capacity'}`,
          suffix: formatFuel(of === 'fuel' ? read.fuelRaw : read.capacityRaw, reply.quantityScale, digits) };
      },
    });
    return [...(options.capacity !== false ? [line('capacity')] : []), line('fuel')];
  }), [views, countries.membership, extent[0], extent[1], single, options.capacity, options.areaFill, tokens, digits]); // eslint-disable-line react-hooks/exhaustive-deps
  const lineSummaries = useMemo(() => prepared.enabled ? new Map(views.flatMap(({ reply }) =>
    (options.capacity === false ? ['fuel'] : ['fuel', 'capacity']).flatMap(measure => {
      const summary = scaledFuelSummary(prepared.summary(reply.tag, measure), reply.quantityScale);
      return summary ? [[`${reply.tag}:${measure}`, summary] as const] : [];
    }))) : undefined, [views, options.capacity, prepared.enabled, prepared.summaries]);
  const strip = useMemo<ChartStrip | undefined>(() => !single || options.strip === false || !views[0] ? undefined
    : barStrip((prepared.enabled ? fuelSummaryStrip(prepared.summary(views[0].reply.tag,'stripGain'), prepared.summary(views[0].reply.tag,'stripBurn'), views[0].reply) : views[0].view.bars).filter(bar => isMember(countries.membership?.get(views[0].reply.tag), bar.lastHour)),
      { height: height => Math.max(CHART_LAYOUT.stripMin, (height - CHART_LAYOUT.stripInset) * STRIP_SHARE), upLabel: 'gain', downLabel: 'burn' }),
    [views, single, options.strip, countries.membership, prepared.summaries]);
  useFuelSeriesParity(anchor, runtime.client, track, countries.tags, extent, countries.membership ?? undefined, 'Fuel chart values', data);
  const colors = { fuel: tokens.accent, capacity: tokens.muted, gained: tokens.categories[0], burned: tokens.categories[1] };
  const state = !track ? <WidgetState kind="empty" icon="droplet" message="Load a campaign and choose countries to see their fuel." />
    : playhead !== null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)
      ? <WidgetState kind="unavailable" message="This time is outside the recording." />
    : groupError || error ? <WidgetState kind="error" failure={groupError ?? error!} onRetry={groupError ? retryGroups : retry} />
      : countries.reason ? <WidgetState kind="empty" message={countries.reason} />
        : (widget.time.mode === 'range' || widget.time.mode === 'pin') && !window ? <WidgetState kind="unavailable" message="Choose another time in widget settings." />
          : pending || !data ? <WidgetState kind="loading" label="Loading fuel…" />
            : !data.some(reply => reply.reads.length) && !prepared.states?.states.some(state=>state.recorded)
              && !prepared.summaries?.series.some(line=>line.summary.buckets.some(bucket=>bucket!==null))
              // A group's members at hours still being prepared are not known yet: none recorded is not proof of none.
              ? countries.known(track.branch.firstHour, track.branch.lastHour) ? <WidgetState kind="unavailable" message="No fuel was recorded for the chosen countries." />
                : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} /> : undefined;
  const open = (hour: number, tag?: string, choices?: readonly string[]) => {
    if (!active || pickRef.current.main !== pickOwner || !pickRef.current.active || saved || state || !track) return;
    hour = Math.floor(hour);
    if (hour < extent[0] || hour > extent[1]) return;
    const choosing = !tag && choices && choices.length > 1;
    setRecipient(previous => fuelRecipientAtPick(previous ?? savedRecipient, choosing ? null : tag ?? chosen?.tag));
    setConsumption(previous => track ? consumptionForActor(previous ?? savedConsumption, track.source.metadata.campaignKey, track.branch.id, choosing ? undefined : tag ?? chosen?.tag) : null);
    if (choosing || tag && tag !== chosen?.tag) setLevel(null);
    setDrillOwner(pickOwner);
    if (choosing) { setChosenTag(undefined); setPickTags(choices); setCountryPick(null); }
    else if (tag && countries.tags.includes(tag) && lines.some(line => line.tag === tag && (prepared.enabled || runAt(line.runs, hour)))) {
      setPickTags(undefined); setChosenTag(tag);
      setCountryPick({ owner: pickOwner, selection: { campaign: track.source.metadata.campaignKey, branch: track.branch.id, tag,
        originOffset: hour - track.branch.firstHour } });
    } else return;
    dispatch(fuelOpen(drillMeasure, hour, choosing ? false : runtime.clock.getSnapshot().playing,
      !choosing && widget.time.mode === 'follow', playhead));
  };
  const main = <CountryRaceView runtime={runtime} track={track} active={active && !saved} options={options} view="lines" extent={extent} playhead={playhead}
    lineSummaries={lineSummaries} onSummaryView={prepared.enabled ? prepared.onSummaryView : undefined}
    onReachHour={prepared.enabled ? prepared.onHover : undefined} navigation={prepared.enabled ? prepared : undefined}
    exactLineTags={prepared.enabled ? async hour => { const data = await prepared.exactAt(hour); return data ? data.states.filter(state=>(state.fuel || state.fuelRaw!==undefined) && isMember(countries.membership?.get(state.tag),hour)).map(state=>state.tag) : null; } : undefined}
    state={state} moment={Math.floor(playhead ?? extent[1])} slots={1} ranked={[]} lines={lines} strip={strip}
    countryTags={countries.tags} keyHours={[...new Set(views.flatMap(item => item.view.readHours))].sort((a, b) => a - b)}
    segmentAt={(tag, hour) => {
      const reads = data?.find(reply => reply.tag === tag)?.reads;
      return reads && runAt(reads, hour) ? reads.filter(read => read.startsSegment && read.firstHour <= hour).at(-1)?.firstHour ?? null : null;
    }}
    label="Fuel" labels={{ lines: 'Fuel level and capacity', actions: 'Fuel actions' }} empty="" formatValue={String}
    presentation={{ countryLooks: !single, held: false, linePadding: CHART_LAYOUT.measureEndLabels }}
    beforeChart={<><p className="chart-line-key">Solid: fuel · dashed: capacity</p>
      <button type="button" className="button small quiet" onClick={() => {
        const hour = Math.floor(playhead ?? extent[1]);
        const tags = [...new Set(lines.filter(line => runAt(line.runs, hour)).map(line => line.tag))];
        if (tags.length) open(hour, tags.length === 1 ? tags[0] : chosenTag, tags);
      }}>View fuel breakdown</button></>}
    onPick={open} pickOwner={pickOwner} tooltip={() => null}
    timeTooltip={single ? hour => isMember(countries.membership?.get(views[0].reply.tag), hour)
      ? fuelTooltips(views[0].reply, views[0].view.reads, hour, dayLabel(hour), colors, digits,prepared.amountsAt(views[0].reply.tag,hour)) : null : undefined}
    lineTooltip={(hour, tags, labels, country) => [{ title: dayLabel(hour), rows: tags.flatMap(tag => {
      const value = views.find(item => item.reply.tag === tag);
      if (!value) return [];
      const label = labels(hour)(tag);
      const tip = fuelTooltips(value.reply, value.view.reads, hour, label.name, colors, digits,prepared.amountsAt(tag,hour))[0];
      return [{ label: label.name, countryFlag: label, swatch: country(tag), strong: true },
        ...tip.rows.map(row => ({ ...row, indent: true })), ...(tip.note ? [{ label: tip.note, tone: 'muted' as const, indent: true }] : [])];
    }) }]} />;
  const wrongSavedActor = chosen && track && savedRecipient && !fuelRecipientForReply(savedRecipient, chosen, track.source.metadata.campaignKey, identitySpans ?? undefined);
  const wrongSavedConsumption = chosen && track && savedConsumption && !consumptionForActor(savedConsumption, track.source.metadata.campaignKey, track.branch.id, chosen.tag, identitySpans ?? undefined);
  const invalidSaved = saved && ((saved.id === 'burn' && options.fuelRecipient && (!savedRecipient || wrongSavedActor))
    || (saved.id === 'consumption' && options.fuelConsumption && (!savedConsumption || wrongSavedConsumption))
    || (saved.id === 'consumption' && options.fuelConsumptionLevel && (!savedLevel || !selectedConsumption && chosen && savedLevel.tag !== chosen.tag))
    || (options.fuelCountry && !savedCountry) || (options.fuelExtent && !heldExtent));
  const actorState = selectedCountry ? catalogue.error && !identityNumbers ? <WidgetState kind="error" failure={catalogue.error} onRetry={catalogue.retry} />
    : !identityNumbers ? <WidgetState kind="loading" label="Loading country…" />
      : !actorSpans ? <WidgetState kind="unavailable" message="Choose a country again." />
        : actorError ? <WidgetState kind="error" failure={actorError} onRetry={actorProduct.retry} />
          : !actorData ? <WidgetState kind="loading" label="Loading the selected country's fuel…" /> : undefined : undefined;
  const headerReason = invalidSaved ? 'Choose a country again.'
    : (catalogue.error && !identityNumbers ? failureText(catalogue.error, 'Couldn’t load country.') : null) ?? (!selectedCountry ? 'Country unavailable at this time.'
      : !identityNumbers ? 'Loading country…'
        : !actorSpans ? 'Choose a country again.'
          : !shownTag ? 'Country unavailable at this time.'
            : (actorError ? failureText(actorError, 'Couldn’t load fuel data.') : null) ?? (!actorData?.some(reply => reply.tag === shownTag && at !== null && runAt(reply.reads, at))
              ? 'The selected country’s fuel is not recorded at this moment.' : null));
  const headerReady = !!saved && !!options.fuelCountry && !!countrySlot && active && headerReason === null && at !== null && !!shownTag;
  const headerWanted = useMemo(() => headerReady && shownTag && at !== null ? [{ tag: shownTag, hour: at }] : [], [headerReady, shownTag, at]);
  const { labelAt: headerLabelAt } = useCountryLabelsAt(runtime.client, track, headerWanted, headerReady);
  const pinnedHeader = saved && options.fuelCountry && countrySlot ? createPortal(<DatedCountryChip failure={(!identityNumbers ? catalogue.error : null) ?? actorError}
    label={headerReady && shownTag && at !== null ? headerLabelAt(at)(shownTag) : null} hour={at} reason={headerReason} />, countrySlot) : null;
  const chart = track && chosen ? {
    active: active && (!!saved || drill.breakdown !== null), pickOwner: detailPickOwner, extent,
    playhead: drill.breakdown !== null && drill.follow ? actualHour : playhead, playback: options.playback as PlaybackMode, afterPlayhead: options.afterPlayhead === true,
    windowHours: Number(options.windowDays ?? 365) * 24, branchStart: track.branch.firstHour, hoursPerSecond: motionOf(runtime.clock.getSnapshot()),
    within: actorWithin ?? countries.membership?.get(chosen.tag), onPick: (hour: number) => {
      if (!pickRef.current.active || pickRef.current.detail !== detailPickOwner) return;
      setDrillOwner(pickOwner); dispatch(saved
      ? { type: 'open', breakdown: saved.id, hour, playing: false, playhead: null } : { type: 'pick', hour });
    },
    onContextMenu: (hour: number, x: number, y: number) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]),
  } : null;
  const breakdown = invalidSaved ? <WidgetState kind="unavailable" message="Choose a country again." /> : actorState ?? state ?? (chosen && track && chart
    ? <>{selectedCountry && !shownTag && <p className="drill-empty">Country unavailable at this time.</p>}
      {drillMeasure === 'consumption' ? <FuelConsumptionBreakdown reply={chosen} actorSpans={actorSpans ?? undefined} identitySpans={identitySpans ?? undefined} runtime={runtime} track={track} hour={detailHour} selection={selectedConsumption} onSelection={setConsumption}
      level={selectedLevel} onLevel={setLevel} digits={digits} {...chart} />
      : <FuelBreakdown reply={chosen} history={selectedCountry ? actorData : undefined} actorSpans={actorSpans ?? undefined} identitySpans={identitySpans ?? undefined}
        measure={drillMeasure} hour={detailHour} tokens={tokens} runtime={runtime} track={track}
        selection={selectedRecipient} onSelection={setRecipient} digits={digits} {...chart} />}</>
    : <p className="drill-empty">Choose a country from the drawn lines for this moment.</p>);
  const refreshNotices = <>{groupRefreshError && <WidgetState kind="error" failure={groupRefreshError} onRetry={retryGroups} />}
    {selectedCountry && catalogue.error && identityNumbers ? <WidgetState kind="error" failure={catalogue.error} onRetry={catalogue.retry} /> : null}</>;
  if (saved) return <div ref={anchor} className="drill-body">
    {pinnedHeader}
    {refreshNotices}
    {(drill.breakdown === null ? widget.time.mode !== 'follow' : !drill.follow) && <button type="button" className="button small quiet" onClick={() => {
      if (!pickRef.current.active || pickRef.current.detail !== detailPickOwner || actorState || invalidSaved) return;
      setDrillOwner(pickOwner); dispatch({ type: 'open', breakdown: saved.id,
        hour: actualHour ?? extent[1], playing: true, playhead: actualHour });
    }}>Follow playback</button>}
    {breakdown}{menu.menu}</div>;
  return <div ref={anchor} style={{ height: '100%', minHeight: 0 }}>{refreshNotices}<DrillHost anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'}
    open={drill.breakdown !== null} title="Fuel consumption and daily accounting" day={at === null ? '' : dayLabel(at)} following={drill.follow} canFollow={actualHour !== null && !!chosen}
    onClose={() => dispatch({ type: 'close' })} onFollow={() => {
      if (!pickRef.current.active || pickRef.current.detail !== detailPickOwner || actorState) return;
      setDrillOwner(pickOwner); dispatch({ type: 'follow' });
    }} main={main}
    onPin={chosen && track && !actorState ? () => {
      if (!pickRef.current.active || pickRef.current.detail !== detailPickOwner) return;
      const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [selectedCountry?.tag ?? chosen.tag] } }, BREAKDOWNS.find(item => item.id === drillMeasure)!, track.id);
      if (!drill.follow && at !== null) pinned.time = { mode: 'pin', windows: { [track.id]: { firstOffset: at - track.branch.firstHour, lastOffset: at - track.branch.firstHour } } };
      if (drill.follow) pinned.time = { mode: 'follow' };
      pinned.options = { ...pinned.options, fuelCountry: selectedCountry ? fuelCountryToken(selectedCountry) : '',
        fuelRecipient: selectedRecipient ? fuelRecipientToken(selectedRecipient) : '',
        fuelConsumption: selectedConsumption ? fuelConsumptionToken(selectedConsumption) : '',
        fuelConsumptionLevel: selectedLevel ? fuelConsumptionLevelToken(selectedLevel) : '',
        fuelExtent: fuelExtentToken(track.source.metadata.campaignKey, track.branch.id, extent) };
      runtime.addWidgetBelow(widget.id, pinned);
    } : undefined}>
    {!single && track && <CountryPicker countries={track.source.metadata.countries} playerCountryId={null} selectedTag={chosen?.tag ?? null}
      source={track.source} client={runtime.client} allowedTags={pickTags ?? countries.tags} flagAt={at !== null ? { branchId: track.branch.id, hour: at } : null}
      onChange={tag => {
        if (!pickRef.current.active || pickRef.current.detail !== detailPickOwner || !tag || at === null
          || !(pickTags ?? countries.tags).includes(tag) || !lines.some(line => line.tag === tag && runAt(line.runs, at))) return;
        setDrillOwner(pickOwner); setChosenTag(tag); setPickTags(undefined);
        setCountryPick({ owner: pickOwner, selection: { campaign: track.source.metadata.campaignKey, branch: track.branch.id,
          tag, originOffset: at - track.branch.firstHour } });
        setRecipient(null); setConsumption(null); setLevel(null);
      }} />}
    <Segmented label="Breakdown of" value={drillMeasure} onChange={next => {
      setMeasure(next); setRecipient(null); setConsumption(null); setLevel(null);
      if (drill.breakdown !== null && at !== null) dispatch({ type: 'open', breakdown: next, hour: at, playing: drill.follow, playhead: drill.follow ? actualHour : null });
    }} options={BREAKDOWNS.map(item => ({ value: item.id, label: item.label }))} />
    {breakdown}
  </DrillHost>{menu.menu}</div>;
}
