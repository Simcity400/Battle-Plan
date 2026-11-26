import type { CountryAt, Gate } from '../../dashboard/regionGate';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import { runAt, type Span } from '../../charts/series';
import type { FamilyViewProps, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetBase, WidgetTime } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import type { Region } from '../../dashboard/regionTime';
import { TimeRegion } from '../../dashboard/widgetTime.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { FamilyViewportChange } from '../../families/host/adapter.ts';
import type { InstalledAction } from '../../families/plan.ts';
import { productRequest } from '../../dashboard/request.ts';
import { dayLabel } from '../../charts/draw.ts';
import type { CountryFuelReply, FuelAwardHistoryReply, FuelAwardPart, FuelAwardsReply } from '../../generated/dashboard';
import { COUNTRY_FUEL, formatFuelPart, LEFT_OUT, validateCountryFuel, validateFuelAwardHistory, validateFuelAwards } from './fuelData';
import { detailWindows, followedHistoryAnchor, fuelRecipientFor, intersectSpans, joinedRuns, readBasis, selectFuelRecipient, type FuelBasis, type FuelRecipientSelection } from './fuelDrill';
import {
  buildRecipientsTableModel,
  FUEL_ACCOUNTING_TABLE_KIND,
  FUEL_DETAIL_TABLE_KIND,
  FUEL_HISTORY_CHART_KIND,
  FUEL_SERIES_CHART_KIND,
  fuelAccountingTableDeclaration,
  fuelDetailTableDeclaration,
  fuelHistoryChartDeclaration,
  fuelSeriesChartDeclaration,
  fuelSeriesLabelsFromReply,
  type FuelAccountingTableOptions,
  type FuelDetailTableOptions,
  type FuelHistoryChartOptions,
  type FuelPartRowMeta,
  type FuelSeriesChartOptions,
} from './fuelDetailProjection.ts';

export interface FuelDrawerShared {
  readonly active: boolean;
  readonly pickOwner: object;
  readonly extent: readonly [number, number];
  readonly playhead: number | null;
  readonly within: readonly Span[] | null;
  readonly windowHours?: number | null;
  readonly detailHours: number;
  readonly region: Region;
  readonly onPick: (hour: number) => void;
}

const EMPTY_MAP_OPTIONS: FamilyViewProps['mapOptions'] = { country: null, fog: false };
const NOOP = () => {};

function pinnedTime(track: BoundTrack, hour: number): WidgetTime {
  const offset = Math.floor(hour) - track.branch.firstHour;
  return { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } };
}

function rangeTime(track: BoundTrack, firstHour: number, lastHour: number): WidgetTime {
  const firstOffset = Math.floor(firstHour) - track.branch.firstHour;
  const lastOffset = Math.max(firstOffset, Math.floor(lastHour) - track.branch.firstHour);
  return { mode: 'range', firstOffset, lastOffset };
}

export function useDetailView(start: number, end: number, detailHours: number) {
  const [zoomed, setZoomed] = useState<readonly [number, number] | null>(null);
  const onViewChange = useCallback((x: readonly [number, number] | null) => setZoomed(before => {
    if (!x) return null;
    if (before) {
      const block = (hour: number) => Math.floor(hour / detailHours);
      if (block(before[0]) === block(x[0]) && block(before[1]) === block(x[1])) return before;
    }
    return x;
  }), [detailHours]);
  const visible = useMemo<readonly [number, number]>(() => zoomed ?? [start, end], [zoomed, start, end]);
  return [visible, onViewChange] as const;
}

export function usePlaybackWindowAnchor(
  clock: WidgetRuntime['clock'],
  active: boolean,
  axisOffset: number,
  extent: readonly [number, number],
  detailHours: number,
): number {
  const subscribe = useCallback((changed: () => void) => {
    if (!active) return () => {};
    return clock.onPublish((prev, next) => {
      if (followedHistoryAnchor(extent, prev.hour + axisOffset, detailHours) !== followedHistoryAnchor(extent, next.hour + axisOffset, detailHours)) {
        changed();
      }
    });
  }, [clock, active, axisOffset, extent[0], extent[1], detailHours]);
  const read = useCallback(
    () => followedHistoryAnchor(extent, clock.getHour() + axisOffset, detailHours),
    [clock, axisOffset, extent[0], extent[1], detailHours],
  );
  return useSyncExternalStore(subscribe, read, read);
}

export default function FuelBreakdown({
  runtime,
  gate,
  widget,
  track,
  tag,
  measure,
  hour,
  follows,
  within,
  windowHours,
  selection: retained,
  onSelection,
  digits,
  campaign,
  aliases,
  detailHours,
  history,
  active,
  pickOwner,
  extent,
  playhead,
  region,
  onPick,
}: {
  readonly runtime: WidgetRuntime;
  readonly gate: Gate;
  readonly widget: WidgetBase;
  readonly track: BoundTrack;
  readonly tag: string;
  readonly measure: 'gain' | 'burn';
  readonly hour: number | null;
  readonly follows: boolean;
  readonly digits: number;
  readonly campaign: string;
  readonly aliases?: ReadonlyMap<string, readonly Span[]>;
  readonly history: CountryAt['history'];
  readonly selection: FuelRecipientSelection | null;
  readonly onSelection: (selection: FuelRecipientSelection | null) => void;
} & FuelDrawerShared) {
  const playing = usePlaybackMotion(runtime.clock, active) !== undefined;
  const day = hour === null ? null : Math.floor(hour);
  const pinHour = day ?? extent[1];
  const chartPin = Math.floor(playhead ?? extent[1]);
  const selected = fuelRecipientFor(retained, campaign, track.branch.id, tag, aliases);
  const [picked, setPicked] = useState<{ readonly consumer: number; readonly name: string; readonly basis: FuelBasis | null } | null>(null);
  useEffect(() => { setPicked(null); }, [pickOwner, measure]);
  const consumer = measure === 'burn' ? (selected?.consumer ?? picked?.consumer ?? null) : null;
  const consumerName = picked?.name ?? 'Consumer';
  const liveAwardHour = gate.recorded(follows ? (playing ? null : playhead) : day);
  const liveAwardTag = liveAwardHour === null ? null : gate.country?.aliasAt(liveAwardHour) ?? tag;
  const liveFuel = useProduct<CountryFuelReply>(
    runtime.client,
    measure === 'burn' && picked !== null && !selected && liveAwardHour !== null && liveAwardTag !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: COUNTRY_FUEL,
          params: { branchId: track.branch.id, country: countryFromKey(liveAwardTag), hour: liveAwardHour },
        })
      : null,
    active,
    validateCountryFuel,
    track.source,
  );
  const liveBasis = liveFuel.data && liveAwardTag !== null && liveFuel.data.tag === liveAwardTag && liveFuel.data.read?.day?.awards
    ? readBasis(track.branch.id, liveAwardTag, liveFuel.data.read)
    : null;
  useEffect(() => {
    if (!picked || liveAwardHour === null || !liveFuel.data || liveFuel.data.tag !== liveAwardTag) return;
    const same = picked.basis?.hour === liveBasis?.hour
      && picked.basis?.tag === liveBasis?.tag
      && picked.basis?.branchId === liveBasis?.branchId;
    if (!same) setPicked(prev => (prev ? { ...prev, basis: liveBasis } : null));
  }, [picked, liveAwardHour, liveFuel.data, liveAwardTag, liveBasis]);
  const liveMissingReason = !liveFuel.data
    ? null
    : !liveFuel.data.read
      ? liveFuel.data.missing ?? 'Not recorded'
      : !liveFuel.data.read.day
        ? LEFT_OUT[liveFuel.data.read.leftOut ?? 'notRecorded'].note
        : !liveFuel.data.read.day.awards
          ? liveFuel.data.read.awardsReason ?? 'Breakdown unavailable: the applied updates of this day are not known.'
          : null;
  const pickedBasis = liveAwardHour === null
    ? (picked?.basis ?? null)
    : liveFuel.data
      ? liveBasis
      : picked?.basis?.hour === liveAwardHour
        ? picked.basis
        : null;
  const basis = selected?.basis ?? pickedBasis;
  const basisAt = gate.recorded(basis?.hour);
  const awards = useProduct<FuelAwardsReply>(
    runtime.client,
    basis && basisAt !== null && measure === 'burn' && consumer !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'country-fuel-awards',
          params: { branchId: basis.branchId, country: countryFromKey(basis.tag), hour: basisAt },
        })
      : null,
    active,
    validateFuelAwards,
    track.source,
  );
  const recipients = awards.data && basis && awards.data.readHour === basis.hour && awards.data.tag === basis.tag ? awards.data : null;
  const recipient = selected && recipients?.recipients.find(part => part.key === selected.key);
  const listed = useMemo(() => (recipients?.recipients ?? []).filter(part => part.kind === consumer), [recipients, consumer]);
  const consumerTotal = recipients && consumer !== null ? BigInt(recipients.receivedRaw[consumer]) : null;
  const gainScale = 100000n;

  const tableWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-accounting:${measure}`,
    kind: FUEL_ACCOUNTING_TABLE_KIND,
    time: follows ? { mode: 'follow' } : pinnedTime(track, pinHour),
  }), [widget, track, measure, follows, pinHour]);

  const incomingTableWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-accounting:incoming`,
    kind: FUEL_ACCOUNTING_TABLE_KIND,
    time: follows ? { mode: 'follow' } : pinnedTime(track, pinHour),
  }), [widget, track, follows, pinHour]);

  const ownChartWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-series:${measure}`,
    kind: FUEL_SERIES_CHART_KIND,
    time: follows ? { mode: 'follow' } : rangeTime(track, extent[0], Math.min(extent[1], chartPin)),
  }), [widget, track, measure, follows, extent[0], extent[1], chartPin]);

  const incomingChartWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-series:incoming`,
    kind: FUEL_SERIES_CHART_KIND,
    time: follows ? { mode: 'follow' } : rangeTime(track, extent[0], Math.min(extent[1], chartPin)),
  }), [widget, track, follows, extent[0], extent[1], chartPin]);

  const recipientsWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-recipients:${consumer ?? 'none'}`,
    kind: FUEL_DETAIL_TABLE_KIND,
    time: pinnedTime(track, basis?.hour ?? pinHour),
  }), [widget, track, consumer, basis?.hour, pinHour]);

  const accountingOptions = useMemo<FuelAccountingTableOptions>(() => ({
    tag,
    mode: measure,
    digits,
    branchId: track.branch.id,
  }), [tag, measure, digits, track.branch.id]);

  const incomingOptions = useMemo<FuelAccountingTableOptions>(() => ({
    tag,
    mode: 'incoming',
    digits,
    branchId: track.branch.id,
  }), [tag, digits, track.branch.id]);

  const chartSpans = useMemo(() => intersectSpans(history?.spans, within), [history?.spans, within]);
  const labelHour = gate.recorded(day ?? history?.spans.at(-1)?.lastHour ?? chartPin);
  const labelTag = labelHour !== null ? gate.country?.aliasAt(labelHour) ?? tag : null;
  const labelFuel = useProduct<CountryFuelReply>(
    runtime.client,
    (measure !== 'burn' || (!selected && consumer === null)) && labelHour !== null && labelTag !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: COUNTRY_FUEL,
          params: { branchId: track.branch.id, country: countryFromKey(labelTag), hour: labelHour },
        })
      : null,
    active,
    validateCountryFuel,
    track.source,
  );
  const ownLabels = useMemo(() => fuelSeriesLabelsFromReply(labelFuel.data, measure, digits), [labelFuel.data, measure, digits]);
  const incomingLabels = useMemo(() => fuelSeriesLabelsFromReply(labelFuel.data, 'incoming', digits), [labelFuel.data, digits]);

  const ownChartOptions = useMemo<FuelSeriesChartOptions>(() => ({
    tag,
    historyKey: history?.key ?? null,
    historySpans: chartSpans,
    mode: measure,
    digits,
    extent,
    selectedHour: follows ? null : day,
    ...(windowHours != null ? { windowHours } : {}),
    labels: ownLabels,
    surface: '',
  }), [tag, history?.key, chartSpans, measure, digits, extent, follows, day, windowHours, ownLabels]);

  const incomingChartOptions = useMemo<FuelSeriesChartOptions>(() => ({
    tag,
    historyKey: history?.key ?? null,
    historySpans: chartSpans,
    mode: 'incoming',
    digits,
    extent,
    selectedHour: follows ? null : day,
    ...(windowHours != null ? { windowHours } : {}),
    labels: incomingLabels,
    surface: '',
  }), [tag, history?.key, chartSpans, digits, extent, follows, day, windowHours, incomingLabels]);

  const recipientsOptions = useMemo<FuelDetailTableOptions | null>(() => (
    recipients && consumerTotal !== null
      ? { model: buildRecipientsTableModel(basis?.hour ?? pinHour, listed, consumerName, consumerTotal, gainScale, digits) }
      : null
  ), [recipients, consumerTotal, basis?.hour, pinHour, listed, consumerName, digits]);

  const handlePartActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as FuelPartRowMeta | undefined;
    if (measure === 'burn' && meta?.consumer !== undefined) {
      setPicked({ consumer: meta.consumer, name: meta.consumerName ?? 'Consumer', basis: meta.basis ?? null });
    }
  }, [measure]);

  const handleRecipientActivate = useCallback((action: InstalledAction) => {
    const found: FuelAwardPart | undefined = listed.find(row => row.key === action.subject);
    if (found && basis) onSelection(selectFuelRecipient(campaign, basis, found));
  }, [listed, basis, campaign, onSelection]);

  const handleChartActivate = useCallback((action: InstalledAction) => {
    onPick(Math.floor(action.hour));
  }, [onPick]);

  return <div className="fuel-detail-section">
    {retained && !selected && <p className="drill-empty">Choose a recipient again.</p>}
    {selected ? <>
      <div className="fuel-detail-bar">
        <button type="button" className="button small quiet" onClick={() => { setPicked({ consumer: selected.consumer, name: consumerName, basis: selected.basis }); onSelection(null); }}>All consumer recipients</button>
        <strong>{recipient?.recordedName ?? selected.name ?? 'Name unavailable'}</strong>
      </div>
      <RecipientHistory
        runtime={runtime}
        gate={gate}
        widget={widget}
        track={track}
        basis={selected.basis}
        recipientKey={selected.key}
        name={recipient?.recordedName ?? selected.name}
        hour={day}
        follows={follows}
        playhead={chartPin}
        extent={extent}
        within={within}
        detailHours={detailHours}
        active={active}
        onPick={onPick}
      />
    </> : consumer !== null && measure === 'burn' ? <>
      <div className="fuel-detail-bar">
        <button type="button" className="button small quiet" onClick={() => setPicked(null)}>All fuel sources and consumers</button>
        <strong>{consumerName}</strong>
        {basis && follows && playing && <span className="drawer-day-label"> · {dayLabel(basis.hour)}</span>}
      </div>
      {!basis && liveAwardHour === null && follows ? <p className="drill-empty">Actual recipients are listed for the day shown once playback stops.</p>
        : !basis && liveFuel.error ? <WidgetState kind="error" what="fuel accounting" failure={liveFuel.error} onRetry={liveFuel.retry} />
        : !basis && !liveFuel.data ? <WidgetState kind="loading" label="Loading actual fuel recipients…" />
        : !basis ? <p className="drill-empty">{liveMissingReason ?? 'Breakdown unavailable: the applied updates of this day are not known.'}</p>
        : awards.error ? <WidgetState kind="error" what="recipients" failure={awards.error} onRetry={awards.retry} />
          : !recipientsOptions ? <WidgetState kind="loading" label="Loading actual fuel recipients…" />
            : <div className="fuel-detail-table fh-frame" style={{ flex: '1 1 280px' }}>
                <FamilyWidget
                  widget={recipientsWidget}
                  options={recipientsOptions as unknown as FamilyViewProps['options']}
                  mapOptions={EMPTY_MAP_OPTIONS}
                  headerSlot={null}
                  onDiagnostics={NOOP}
                  runtime={runtime}
                  active={active}
                  family="table"
                  declaration={fuelDetailTableDeclaration}
                  onActivate={handleRecipientActivate}
                />
              </div>}
    </> : <>
      <TimeRegion runtime={runtime} region={region}>
        <div className="fuel-detail-table fh-frame">
          <FamilyWidget
            widget={tableWidget}
            options={accountingOptions as unknown as FamilyViewProps['options']}
            mapOptions={EMPTY_MAP_OPTIONS}
            headerSlot={null}
            onDiagnostics={NOOP}
            runtime={runtime}
            active={active}
            family="table"
            declaration={fuelAccountingTableDeclaration}
            onActivate={measure === 'burn' ? handlePartActivate : undefined}
          />
        </div>
        {measure === 'gain' && (
          <div className="fuel-detail-table-single fh-frame">
            <FamilyWidget
              widget={incomingTableWidget}
              options={incomingOptions as unknown as FamilyViewProps['options']}
              mapOptions={EMPTY_MAP_OPTIONS}
              headerSlot={null}
              onDiagnostics={NOOP}
              runtime={runtime}
              active={active}
              family="table"
              declaration={fuelAccountingTableDeclaration}
            />
          </div>
        )}
      </TimeRegion>
      <div className="fuel-detail-chart fh-frame">
        <FamilyWidget
          widget={ownChartWidget}
          options={ownChartOptions as unknown as FamilyViewProps['options']}
          mapOptions={EMPTY_MAP_OPTIONS}
          headerSlot={null}
          onDiagnostics={NOOP}
          runtime={runtime}
          active={active}
          family="time-chart"
          declaration={fuelSeriesChartDeclaration}
          onActivate={handleChartActivate}
        />
      </div>
      {measure === 'gain' && (
        <div className="fuel-detail-chart fh-frame">
          <FamilyWidget
            widget={incomingChartWidget}
            options={incomingChartOptions as unknown as FamilyViewProps['options']}
            mapOptions={EMPTY_MAP_OPTIONS}
            headerSlot={null}
            onDiagnostics={NOOP}
            runtime={runtime}
            active={active}
            family="time-chart"
            declaration={fuelSeriesChartDeclaration}
            onActivate={handleChartActivate}
          />
        </div>
      )}
    </>}
  </div>;
}

function RecipientHistory({
  runtime,
  gate,
  widget,
  track,
  basis,
  recipientKey,
  name,
  hour,
  follows,
  playhead,
  extent,
  within,
  detailHours,
  active,
  onPick,
}: {
  readonly runtime: WidgetRuntime;
  readonly gate: Gate;
  readonly widget: WidgetBase;
  readonly track: BoundTrack;
  readonly basis: FuelBasis;
  readonly recipientKey: string;
  readonly name: string | null;
  readonly hour: number | null;
  readonly follows: boolean;
  readonly playhead: number;
  readonly extent: readonly [number, number];
  readonly within: readonly Span[] | null;
  readonly windowHours?: number | null;
  readonly detailHours: number;
  readonly active: boolean;
  readonly onPick: (hour: number) => void;
}) {
  const playing = usePlaybackMotion(runtime.clock, active && follows) !== undefined;
  const liveAnchor = usePlaybackWindowAnchor(runtime.clock, active && follows && playing, track.axisOffset, extent, detailHours);
  const anchorHour = follows ? (playing ? liveAnchor : playhead) : (hour ?? playhead);
  const end = Math.max(extent[0], Math.min(extent[1], anchorHour));
  const start = Math.max(extent[0], end - detailHours + 1);
  const [visible, onViewChange] = useDetailView(start, end, detailHours);
  const wanted = useMemo(() => detailWindows(extent, visible, anchorHour, detailHours), [extent[0], extent[1], visible[0], visible[1], anchorHour, detailHours]);
  const basisAt = gate.recorded(basis.hour);
  const windows = useProductWindows<FuelAwardHistoryReply>(
    runtime.client,
    basisAt === null ? null : JSON.stringify([track.source.id, track.source.revision, basis, recipientKey]),
    window => productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'country-fuel-award-history',
      params: { branchId: basis.branchId, country: countryFromKey(basis.tag), hour: basisAt!, window, recipientKey },
    }),
    wanted,
    active,
    validateFuelAwardHistory,
    track.source,
  );
  const runs = useMemo(() => joinedRuns([...windows.data.values()].map(reply => reply.runs)), [windows.data]);
  const current = hour === null ? null : runAt(runs, hour);
  const currentText = hour === null
    ? 'No selected moment'
    : within && !runAt(within, hour)
      ? 'Outside the selected group membership'
      : current?.amountRaw == null
        ? current?.reason ?? 'History unavailable'
        : formatFuelPart(BigInt(current.amountRaw), 100000n);

  const chartWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-recipient-history:${recipientKey}`,
    kind: FUEL_HISTORY_CHART_KIND,
    time: follows ? { mode: 'follow' } : pinnedTime(track, playhead),
  }), [widget, track, recipientKey, follows, playhead]);

  const loading = active && basisAt !== null && wanted.some(window => !windows.data.has(window) && !windows.errors.has(window));
  const failed = windows.errors.size > 0;

  const chartOptions = useMemo<FuelHistoryChartOptions>(() => ({
    label: `${name ?? 'Recipient'}: fuel receipts over time`,
    series: [{ id: recipientKey, name: name ?? 'Other', colorIndex: 0, runs }],
    extent,
    viewX: [start, end],
    selectedHour: follows ? null : hour,
    within,
    minZero: true,
    ...(loading ? { pending: true } : {}),
    ...(failed ? { failed: true } : {}),
    surface: '',
  }), [name, recipientKey, runs, extent, start, end, follows, hour, within, loading, failed]);

  const handleViewport = useCallback((vp: FamilyViewportChange) => {
    onViewChange(vp.window ?? null);
  }, [onViewChange]);

  const handleActivate = useCallback((action: InstalledAction) => {
    onPick(Math.floor(action.hour));
  }, [onPick]);

  return <>
    <p className="drill-empty">{name ?? 'Name unavailable'}: {currentText}</p>
    {loading && <p className="drill-empty">Loading recipient history…</p>}
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" what="recipient history" failure={reason} onRetry={windows.retry} />)}
    <div className="fuel-detail-chart fh-frame">
      <FamilyWidget
        widget={chartWidget}
        options={chartOptions as unknown as FamilyViewProps['options']}
        mapOptions={EMPTY_MAP_OPTIONS}
        headerSlot={null}
        onDiagnostics={NOOP}
        runtime={runtime}
        active={active}
        family="time-chart"
        declaration={fuelHistoryChartDeclaration}
        onViewportChange={handleViewport}
        onActivate={handleActivate}
      />
    </div>
  </>;
}
