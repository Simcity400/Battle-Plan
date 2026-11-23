import { failureDetails } from '../../dashboard/queryError.ts';
import { useCallback, useEffect, useMemo,  useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import {  pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { jumpItem } from '../../charts/jump';
import { usNumber } from '../../charts/numberFormat';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveOptions } from '../../dashboard/options';
import type { OptionValue } from '../../dashboard/options';
import { peaceDealAt } from '../../dashboard/peaceDealsData';
import { useHeadHour } from '../army-size/headHour.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { COUNTRY_SORT, OPTIONS, TRANSFER_BREAKDOWN, TRANSFER_SORT, TRANSFER_SORT_KEY } from './definition';
import TransferDrill from './TransferDrill';
import { CountryTable, TransferTable } from './TransferTables';
import { selectedTransfers, transferSelection, transferSelectionToken, transferTime } from './transferDrillData';
import { chosenConference, conferenceChoiceId, conferenceToken, countryBalances, decodeTransfers, involving, transferConferenceLabel, transferCutoff, transferEmptyMessage,
  transferGapHint, transferGapWindow, transferGaps, transferHow, transferScope, transfersAt, visibleConferences } from './territoryData.ts';
import type { CountryBalance, TransferOccurrence } from './territoryData';
import { countryTooltip, transferRequest, transferTooltip } from './territoryTooltip';
import type { TransferLabels, TransferTipRequest } from './territoryTooltip';
import './territoryTransfers.css';
import { motionOf } from '../../charts/easing.ts';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { axisAt } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: string[] = [];
const NO_TRANSFERS: readonly TransferOccurrence[] = [];
const NO_BALANCES: readonly CountryBalance[] = [];
const NO_REQUESTS: TransferTipRequest = { countries: [], states: [] };

export default function TerritoryTransfersWidget({ widget, runtime, active, onDiagnostics, headerSlot, setOption }: WidgetProps) {
  const savedView = widget.options?.show === TRANSFER_BREAKDOWN.id;
  const drillControl = useDrill(runtime, useRegion(widget), savedView ? TRANSFER_BREAKDOWN.id : null);
  const drill = drillControl.state;
  const dispatchDrill = drillControl.dispatch;
  const [drillCountry, setDrillCountry] = useState<string | null>(null);
  const drillAnchor = useRef<HTMLDivElement>(null);
  const following = widget.time.mode === 'follow';
  const { track, data, error, retry } = usePeaceDeals(widget, runtime, active);
useShownEachFrame(runtime.clock, () => data && track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const transfers = useMemo(() => data && decodeTransfers(data), [data]);
  const options = resolveOptions(OPTIONS, widget.options);
  const view = options.show === 'transfers' || options.show === TRANSFER_BREAKDOWN.id ? options.show : 'main';
  const pinned = view === TRANSFER_BREAKDOWN.id;
  const scope = options.scope === 'conference' ? 'conference' : 'campaign';
  const selectedToken = String(options.conference ?? '');
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const selectedId = track ? conferenceChoiceId(selectedToken, campaignKey, track.branch.id) : null;
  const storedSelection = useMemo(() => track ? transferSelection(String(options.selection ?? ''), campaignKey, track.branch.id) : null,
    [options.selection, track, campaignKey]);
  const playbackHour = useHeadHour(runtime.clock, active && (following || drill.breakdown != null && drillControl.following), at => {
    if (!track || !transfers) return '';
    const reach = transferTime(widget, track, runtime.tracks, at, pinned ? storedSelection?.fromOffset : null).moment;
    return `${transferCutoff(transfers.hours, reach)}:${transferCutoff(transfers.conferenceHours, reach)}`;
  });
  const time = track ? transferTime(widget, track, runtime.tracks, playbackHour, pinned ? storedSelection?.fromOffset : null) : { moment: -Infinity, from: -Infinity };
  const { moment, from: rangeFrom } = time;
  const wholeCampaign = widget.time.mode === 'campaign';
  const conferenceThrough = transfers ? transferCutoff(transfers.conferenceHours, moment) : -Infinity;
  const available = useMemo(() => data ? visibleConferences(data.deals, conferenceThrough, rangeFrom) : [], [data, conferenceThrough, rangeFrom]);
  const conference = useMemo(() => {
    const selected = scope === 'conference' && selectedId != null ? chosenConference(available, selectedId) : null;
    return selected ? peaceDealAt(selected, moment) : null;
  }, [scope, available, selectedId, moment]);
  const conferenceId = scope === 'conference' ? conference?.id ?? '' : null;
  const scoped = useMemo(() => transfers ? transferScope(transfers, conferenceId) : NO_TRANSFERS, [transfers, conferenceId]);
  const through = transfers ? transferCutoff(transfers.hours, moment) : -Infinity;
  const reached = useMemo(() => transfersAt(scoped, through, rangeFrom, wholeCampaign), [scoped, through, rangeFrom, wholeCampaign]);

  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const tagsKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(tagsKey), [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const shown = useMemo(() => !filtered ? reached : !match.ready ? NO_TRANSFERS
    : reached.filter(occurrence => match.matches(occurrence.to.tag, occurrence.to.hour) || occurrence.from != null && match.matches(occurrence.from.tag, occurrence.from.hour)),
  [filtered, reached, match]);
  const balances = useMemo(() => {
    const rows = countryBalances(shown);
    return filtered ? rows.filter(row => match.matches(row.side.tag, row.side.hour)) : rows;
  }, [shown, filtered, match]);
  const selectionMissing = useMemo(() => pinned && storedSelection ? selectedTransfers(scoped, storedSelection).missing : 0, [pinned, storedSelection, scoped]);

  useEffect(() => {
    onDiagnostics(data?.warnings ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  const [visibleTransfers, setVisibleTransfers] = useState<readonly TransferOccurrence[]>(NO_TRANSFERS);
  const [visibleCountries, setVisibleCountries] = useState<readonly CountryBalance[]>(NO_BALANCES);
  const [requested, setRequested] = useState<TransferTipRequest>(NO_REQUESTS);
  const requestKey = useRef('');
  const ask = useCallback((next: TransferTipRequest) => {
    const key = JSON.stringify(next);
    if (key === requestKey.current) return;
    requestKey.current = key;
    queueMicrotask(() => setRequested(next));
  }, []);
  useEffect(() => { if (!active) { requestKey.current = ''; setRequested(NO_REQUESTS); } }, [active]);
  const wantedKey = useMemo(() => {
    const rows = transferRequest(visibleTransfers, visibleCountries);
    return JSON.stringify({ countries: [...rows.countries, ...requested.countries], states: [...rows.states, ...requested.states] });
  }, [visibleTransfers, visibleCountries, requested]);
  const wanted = useMemo<TransferTipRequest>(() => JSON.parse(wantedKey), [wantedKey]);
  const maxHours = Math.max(MAX_NAME_HOURS, new Set(wanted.countries.map(country => country.hour)).size);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted.countries, active, maxHours);
  const stateAt = useStateNamesAt(runtime.client, track, wanted.states, active);
  const labels = useMemo<TransferLabels>(() => ({
    country: side => labelAt(side.hour)(side.tag),
    tag: (tag, hour) => labelAt(hour)(tag),
    state: occurrence => stateAt(occurrence.afterHour, occurrence.state).name ?? `State ${occurrence.state}`,
  }), [labelAt, stateAt]);
  const countryTip = useCallback((row: CountryBalance) => {
    ask(transferRequest(NO_TRANSFERS, [row]));
    return countryTooltip(row, labels);
  }, [ask, labels]);
  const transferTip = useCallback((row: TransferOccurrence) => {
    const deal = transfers?.deals.get(row.deal);
    const request = transferRequest([row]);
    const overlord = transferHow(row, deal).subjectOf;
    if (overlord != null) request.countries.push({ tag: overlord, hour: row.afterHour });
    ask(request);
    return transferTooltip(row, deal, labels);
  }, [ask, labels, transfers]);
  const countryMenu = useCallback((row: CountryBalance) => track ? [...new Set([row.last, row.first])].map(hour => jumpItem(runtime, track.id, hour)) : [], [runtime, track]);
  const transferMenu = useCallback((row: TransferOccurrence) => track ? [jumpItem(runtime, track.id, row.afterHour)] : [], [runtime, track]);

  const playhead = track ? Math.floor(playbackHour + track.axisOffset) : null;
  const drillRegion = drillControl.region;
  const regionAt = useRegionHour(runtime, drillRegion, active && drill.breakdown != null);
  const followedAt = drill.breakdown && drillControl.following ? drillControl.gate.hourAt(axisAt(playbackHour)) : null;
  const drillAt = drill.breakdown ? followedAt ?? regionAt ?? moment : moment;
  const drillThrough = transfers ? transferCutoff(transfers.hours, drillAt) : -Infinity;
  const drillReached = useMemo(() => drill.breakdown || pinned ? transfersAt(scoped, drillThrough, rangeFrom, wholeCampaign) : NO_TRANSFERS,
    [drill.breakdown, pinned, scoped, drillThrough, rangeFrom, wholeCampaign]);
  const drillRows = useMemo(() => pinned ? storedSelection ? selectedTransfers(drillReached, storedSelection).shown : NO_TRANSFERS
    : drillCountry != null ? involving(drillReached, drillCountry) : NO_TRANSFERS, [pinned, storedSelection, drillReached, drillCountry]);
  const focus = pinned ? storedSelection?.country ?? null : drillCountry;
  const openDrill = useCallback((row: CountryBalance) => {
    setDrillCountry(row.key);
    drillControl.show('parts', moment);
  }, [moment, drillControl]);
  useEffect(() => { dispatchDrill({ type: 'close' }); setDrillCountry(null); }, [track?.source.id, track?.branch.id, scope, selectedToken]);
  const transferSetOption = useMemo(() => setOption && ((key: string, value: OptionValue) =>
    setOption(key === 'tableSort' ? TRANSFER_SORT_KEY : key, value)), [setOption]);
  const tables = { labels, cellBars: options.cellBars !== false, density: options.density === 'compact' ? 'compact' as const : 'comfortable' as const, narrow: narrowTableOf(options),
    hoursPerSecond: motionOf(runtime.clock.getSnapshot()) };
  const transferSort = tableSortOf(options[TRANSFER_SORT_KEY], TRANSFER_SORT);

  if (!track) return <WidgetState kind="empty" icon="flows" message="Load a campaign to see territory transfers." />;
  const parentOutside = moment < track.branch.firstHour || moment > track.branch.lastHour;
  if (parentOutside && !drillControl.open) return <WidgetState kind="outside" />;
  const failure = error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : null;
  if (failure && !data) return failure;
  if (!data || !transfers) return <WidgetState kind="loading" label="Loading territory transfers…" />;
  if (pinned && selectionMissing) return failure ?? <WidgetState kind="unavailable" message={`${usNumber.format(selectionMissing)} selected transfer links are not recorded in this scope.`} />;
  if (scope === 'conference' && !conference) return failure ?? <WidgetState kind="empty" message={selectedToken ? 'This conference is outside the selected recorded interval.' : 'No peace conferences yet.'} />;
  if (filtered && !match.ready) return <>{failure}{groups.error && !groups.data
    ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
    : <WidgetState kind="loading" label="Loading territory transfers…" />}</>;

  const [gapFrom, until] = transferGapWindow(conference, scoped, track.branch.firstHour, moment);
  const gapHint = transferGapHint(transferGaps(data, Math.max(rangeFrom, gapFrom), until));
  const count = pinned ? drillRows.length : shown.length;
  const header = headerSlot && count > 0 && createPortal(<span className="pill">{usNumber.format(count)} {count === 1 ? 'transfer' : 'transfers'}</span>, headerSlot);
  const notes = <>
    {conference && <p className="transfer-note">{transferConferenceLabel(conference)}</p>}
    {gapHint && <p className="transfer-note has-hint" data-tooltip={gapHint}>Some transfers may be missing.</p>}
  </>;
  const first = Math.max(track.branch.firstHour, rangeFrom);
  const empty = <WidgetState kind="empty" message={transferEmptyMessage(filtered)} />;
  const last = widget.time.mode === 'range' ? moment : track.branch.lastHour;
  const drillBody = <TransferDrill rows={drillRows} focus={focus} named={pinned} outside={drillAt < first || drillAt > last}
    {...tables} sort={pinned ? transferSort : TRANSFER_SORT} setOption={pinned ? transferSetOption : undefined}
    tooltip={transferTip} menu={transferMenu} onVisibleRows={setVisibleTransfers} />;

  if (pinned) return <>{failure}<div data-detail={failureDetails(wanted.states.map(read => stateAt(read.hour, read.state).failure))} className="territory-transfers">
    {header}
    {storedSelection == null ? <WidgetState kind="unavailable" message="This transfer selection is unavailable. Select transfers again." />
      : <DrillHost drill={drillControl} runtime={runtime} mode="inline" anchor={drillAnchor} title={TRANSFER_BREAKDOWN.label} day={formatDateTime(drillAt)} canFollow main={null}>
        {notes}{drillBody}</DrillHost>}
  </div></>;

  if (view === 'transfers' && parentOutside) return <WidgetState kind="outside" />;
  if (view === 'transfers') return <>{failure}<div data-detail={failureDetails(wanted.states.map(read => stateAt(read.hour, read.state).failure))} className="territory-transfers">
    {header}{notes}
    {shown.length ? <TransferTable {...tables} rows={shown} sort={transferSort} setOption={transferSetOption} tooltip={transferTip} menu={transferMenu}
      onVisibleRows={setVisibleTransfers} label="Territory transfers" /> : empty}
  </div></>;

  const drillLabel = drillCountry == null ? null : balances.find(row => row.key === drillCountry)?.side ?? drillRows.find(row => row.to.key === drillCountry)?.to
    ?? drillRows.find(row => row.from?.key === drillCountry)?.from ?? null;
  const pin = () => {
    const time = drillControl.time();
    if (!time) return;
    const spec = pinnedBreakdown(widget, TRANSFER_BREAKDOWN, track.id, time);
    runtime.addWidgetBelow(widget.id, { ...spec,
      options: { ...spec.options, scope, conference: conference == null ? '' : conferenceToken(campaignKey, track.branch.id, conference.id),
        selection: transferSelectionToken(campaignKey, track.branch.id, [], Number.isFinite(rangeFrom) ? rangeFrom - track.branch.firstHour : null, drillCountry) } });
  };
  const main = parentOutside ? <WidgetState kind="outside" /> : <div data-detail={failureDetails(wanted.states.map(read => stateAt(read.hour, read.state).failure))} className="territory-transfers">
    {notes}
    {balances.length ? <CountryTable {...tables} rows={balances} sort={tableSortOf(options.tableSort, COUNTRY_SORT)} setOption={setOption}
      onPick={openDrill} tooltip={countryTip} menu={countryMenu} onVisibleRows={setVisibleCountries} /> : empty}
  </div>;
  return <>{failure}<div data-detail={failureDetails(wanted.states.map(read => stateAt(read.hour, read.state).failure))} className="territory-transfer-host" ref={drillAnchor}>
    {header}
    <DrillHost drill={drillControl} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={drillAnchor}
      title={drillLabel ? labels.country(drillLabel).name : 'Country transfers'} day={formatDateTime(drillAt)} canFollow={playhead != null}
      returnFocus={drillCountry == null || typeof CSS === 'undefined' ? undefined : `tr[data-table-row="${CSS.escape(drillCountry)}"]`}
      onClose={() => dispatchDrill({ type: 'close' })} pinRefused={drillControl.pinRefused} onPin={pin} main={main}>{drillBody}</DrillHost>
  </div></>;
}
