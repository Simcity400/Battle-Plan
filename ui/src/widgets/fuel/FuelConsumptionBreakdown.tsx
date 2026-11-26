import type { CountryAt, Gate } from '../../dashboard/regionGate';
import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import type { Span } from '../../charts/series';
import type { FamilyViewProps, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetBase, WidgetTime } from '../../dashboard/model';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useProductWindows } from '../../dashboard/productWindows';
import { productRequest } from '../../dashboard/request.ts';
import { TimeRegion } from '../../dashboard/widgetTime.ts';
import type { FamilyViewportChange } from '../../families/host/adapter.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import type { InstalledAction } from '../../families/plan.ts';
import type { CountryFuelReply, FuelDemandHistoryReply, FuelDemandReply } from '../../generated/dashboard';
import {
  consumptionLevelAvailable,
  consumptionParts,
  demandBasis,
  demandHistoryRange,
  levelComplete,
  validateFuelDemand,
  validateFuelDemandHistory,
  type FuelConsumptionLevel,
  type FuelConsumptionSelection,
} from './fuelConsumption';
import { COUNTRY_FUEL, validateCountryFuel } from './fuelData';
import { detailWindows, fuelRecipientFor, intersectSpans, joinedRuns, type FuelBasis } from './fuelDrill';
import { useDetailView, usePlaybackWindowAnchor, type FuelDrawerShared } from './FuelBreakdown';
import {
  buildConsumersTableModel,
  buildTopologyTableModel,
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

export default function FuelConsumptionBreakdown({
  runtime,
  gate,
  widget,
  track,
  tag,
  hour,
  follows,
  within,
  windowHours,
  selection,
  onSelection,
  level,
  onLevel,
  initialLevel,
  digits,
  campaign,
  aliases,
  detailHours,
  history,
  active,
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
  readonly hour: number | null;
  readonly follows: boolean;
  readonly digits: number;
  readonly campaign: string;
  readonly aliases?: ReadonlyMap<string, readonly Span[]>;
  readonly history: CountryAt['history'];
  readonly selection: FuelConsumptionSelection | null;
  readonly onSelection: (selection: FuelConsumptionSelection | null) => void;
  readonly level: FuelConsumptionLevel | null;
  readonly onLevel: (level: FuelConsumptionLevel | null) => void;
  readonly initialLevel?: string | null;
} & FuelDrawerShared) {
  const [showTopology, setShowTopology] = useState(false);
  const [showBranches, setShowBranches] = useState(false);
  const playing = usePlaybackMotion(runtime.clock, active) !== undefined;
  const day = hour === null ? null : Math.floor(hour);
  const pinHour = day ?? extent[1];
  const chartPin = Math.floor(playhead ?? extent[1]);
  const demandHour = gate.recorded(follows ? (playing ? null : playhead) : day);
  const demandTag = demandHour === null ? null : gate.country?.aliasAt(demandHour) ?? null;
  const demand = useProduct<FuelDemandReply>(
    runtime.client,
    demandHour !== null && demandTag !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'country-fuel-demand',
          params: { branchId: track.branch.id, country: countryFromKey(demandTag), hour: demandHour },
        })
      : null,
    active,
    validateFuelDemand,
    track.source,
  );
  const data = demand.data && answersKey(demand.data.asked, demandTag) ? demand.data : undefined;
  const basis = data ? demandBasis(track.branch.id, data) : null;
  const levelName = level && data && level.basis.country === data.country && level.basis.countryInstance === data.countryInstance
    ? level.level
    : level ? null : (initialLevel ?? 'all');
  const available = levelName !== null && (!data || consumptionLevelAvailable(data, levelName));
  const displayed = useMemo(() => (data && available && levelName ? consumptionParts(data, levelName) : []), [data, available, levelName]);
  const complete = !!data && !!levelName && available && levelComplete(data, levelName, displayed);

  const chooseLevel = useCallback((next: string) => {
    onSelection(null);
    if (next === 'all') onLevel(null);
    else if (basis) onLevel({ campaign, branch: track.branch.id, basis, level: next });
  }, [onSelection, onLevel, basis, campaign, track.branch.id]);

  const chosen = fuelRecipientFor(selection, campaign, track.branch.id, tag, aliases);
  const part = chosen && data && data.country === chosen.basis.country && data.countryInstance === chosen.basis.countryInstance
    ? data.parts.find(row => row.key === chosen.key)
    : null;

  const familiesWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-consumption-families`,
    kind: FUEL_ACCOUNTING_TABLE_KIND,
    time: follows ? { mode: 'follow' } : pinnedTime(track, pinHour),
  }), [widget, track, follows, pinHour]);

  const familiesOptions = useMemo<FuelAccountingTableOptions>(() => ({
    tag,
    mode: 'consumption',
    digits,
    branchId: track.branch.id,
  }), [tag, digits, track.branch.id]);

  const consumersWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-consumption-consumers:${levelName ?? 'none'}`,
    kind: FUEL_DETAIL_TABLE_KIND,
    time: pinnedTime(track, demandHour ?? pinHour),
  }), [widget, track, levelName, demandHour, pinHour]);

  const consumersOptions = useMemo<FuelDetailTableOptions | null>(() => (
    data && available ? { model: buildConsumersTableModel(demandHour ?? pinHour, displayed, complete, digits) } : null
  ), [data, available, demandHour, pinHour, displayed, complete, digits]);

  const topologyWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-consumption-topology`,
    kind: FUEL_DETAIL_TABLE_KIND,
    time: pinnedTime(track, demandHour ?? pinHour),
  }), [widget, track, demandHour, pinHour]);

  const topologyOptions = useMemo<FuelDetailTableOptions | null>(() => (
    data ? { model: buildTopologyTableModel(demandHour ?? pinHour, data) } : null
  ), [data, demandHour, pinHour]);

  const branchChartWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-series:branches`,
    kind: FUEL_SERIES_CHART_KIND,
    time: follows ? { mode: 'follow' } : rangeTime(track, extent[0], Math.min(extent[1], chartPin)),
  }), [widget, track, follows, extent[0], extent[1], chartPin]);

  const chartSpans = useMemo(() => intersectSpans(history?.spans, within), [history?.spans, within]);
  const labelHour = gate.recorded(day ?? history?.spans.at(-1)?.lastHour ?? chartPin);
  const labelTag = labelHour !== null ? gate.country?.aliasAt(labelHour) ?? tag : null;
  const labelFuel = useProduct<CountryFuelReply>(
    runtime.client,
    showBranches && !chosen && labelHour !== null && labelTag !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: COUNTRY_FUEL,
          params: { branchId: track.branch.id, country: countryFromKey(labelTag), hour: labelHour },
        })
      : null,
    active && showBranches,
    validateCountryFuel,
    track.source,
  );
  const branchLabels = useMemo(() => fuelSeriesLabelsFromReply(labelFuel.data, 'branches', digits), [labelFuel.data, digits]);

  const branchChartOptions = useMemo<FuelSeriesChartOptions>(() => ({
    tag,
    historyKey: history?.key ?? null,
    historySpans: chartSpans,
    mode: 'branches',
    digits,
    extent,
    selectedHour: follows ? null : day,
    ...(windowHours != null ? { windowHours } : {}),
    labels: branchLabels,
    surface: '',
  }), [tag, history?.key, chartSpans, digits, extent, follows, day, windowHours, branchLabels]);

  const handleFamilyActivate = useCallback((action: InstalledAction) => {
    const meta = action.meta as FuelPartRowMeta | undefined;
    if (meta?.level) chooseLevel(meta.level);
  }, [chooseLevel]);

  const handleConsumerActivate = useCallback((action: InstalledAction) => {
    const row = data?.parts.find(item => item.key === action.subject);
    if (row && basis) onSelection({ campaign, branch: track.branch.id, basis, key: row.key, name: row.recordedName });
  }, [data, basis, campaign, track.branch.id, onSelection]);

  const handleTopologyActivate = useCallback((action: InstalledAction) => {
    chooseLevel(action.subject);
  }, [chooseLevel]);

  const handleChartActivate = useCallback((action: InstalledAction) => {
    onPick(Math.floor(action.hour));
  }, [onPick]);

  const displayedKeys = useMemo(() => displayed.map(row => row.key), [displayed]);
  const displayedNames = useMemo(() => new Map(displayed.map(row => [row.key, row.recordedName])), [displayed]);
  const displayedColors = useMemo(() => new Map(displayed.map(row => [row.key, row.kind])), [displayed]);

  return <div className="fuel-detail-section">
    <TimeRegion runtime={runtime} region={region}>
      <div className="fuel-detail-table-compact fh-frame">
        <FamilyWidget
          widget={familiesWidget}
          options={familiesOptions as unknown as FamilyViewProps['options']}
          mapOptions={EMPTY_MAP_OPTIONS}
          headerSlot={null}
          onDiagnostics={NOOP}
          runtime={runtime}
          active={active}
          family="table"
          declaration={fuelAccountingTableDeclaration}
          onActivate={handleFamilyActivate}
        />
      </div>
    </TimeRegion>
    {demand.error && <WidgetState kind="error" what="consumers" failure={demand.error} onRetry={demand.retry} />}
    {chosen ? <>
      <div className="fuel-detail-bar">
        <button type="button" className="button small quiet" onClick={() => onSelection(null)}>Back to displayed consumers</button>
        <strong>{part?.recordedName ?? chosen.name ?? 'Name unavailable'}</strong>
      </div>
      {part?.reason && <p className="drill-empty">Consumption unavailable: {part.reason}</p>}
      {data && !part && <p className="drill-empty">Consumer unavailable at this time.</p>}
      <DemandHistory
        runtime={runtime}
        gate={gate}
        widget={widget}
        track={track}
        basis={chosen.basis}
        keys={[chosen.key]}
        names={new Map([[chosen.key, part?.recordedName ?? chosen.name]])}
        colorIndexes={new Map([[chosen.key, 0]])}
        hour={day}
        anchorHour={follows ? chartPin : (day ?? demandHour ?? chosen.basis.hour)}
        follows={follows}
        playhead={chartPin}
        extent={extent}
        within={within}
        detailHours={detailHours}
        active={active}
        onPick={onPick}
        label="Individual current consumption required per day"
      />
    </> : <>
      {demandHour === null ? <p className="drill-empty">{follows ? 'Individual consumers are listed for the day shown once playback stops.' : 'Individual consumption was not recorded for this moment.'}</p>
        : !data && !demand.error ? <WidgetState kind="loading" label="Loading individual consumers…" /> : null}
      {levelName !== 'all' && <div className="fuel-detail-bar"><button type="button" className="button small quiet" onClick={() => chooseLevel('all')}>All current consumers</button></div>}
      {data && !available && <p className="drill-empty">Not recorded at this moment.</p>}
      {consumersOptions && (
        <div className="fuel-detail-table fh-frame">
          <FamilyWidget
            widget={consumersWidget}
            options={consumersOptions as unknown as FamilyViewProps['options']}
            mapOptions={EMPTY_MAP_OPTIONS}
            headerSlot={null}
            onDiagnostics={NOOP}
            runtime={runtime}
            active={active}
            family="table"
            declaration={fuelDetailTableDeclaration}
            onActivate={handleConsumerActivate}
          />
        </div>
      )}
      {topologyOptions && (
        <details className="fuel-detail-details" open={showTopology} onToggle={e => setShowTopology(e.currentTarget.open)}>
          <summary>Armies, fleets and air bases</summary>
          {showTopology && (
            <div className="fuel-detail-table fh-frame">
              <FamilyWidget
                widget={topologyWidget}
                options={topologyOptions as unknown as FamilyViewProps['options']}
                mapOptions={EMPTY_MAP_OPTIONS}
                headerSlot={null}
                onDiagnostics={NOOP}
                runtime={runtime}
                active={active && showTopology}
                family="table"
                declaration={fuelDetailTableDeclaration}
                onActivate={handleTopologyActivate}
              />
            </div>
          )}
        </details>
      )}
      {basis && data && available && displayed.length > 0 && (
        <DemandHistory
          runtime={runtime}
          gate={gate}
          widget={widget}
          track={track}
          basis={basis}
          keys={displayedKeys}
          names={displayedNames}
          colorIndexes={displayedColors}
          hour={day}
          anchorHour={follows ? chartPin : (day ?? demandHour ?? basis.hour)}
          follows={follows}
          playhead={chartPin}
          extent={extent}
          within={within}
          detailHours={detailHours}
          active={active}
          onPick={onPick}
          label="Displayed individual current consumption required per day"
        />
      )}
      <details className="fuel-detail-details" open={showBranches} onToggle={e => setShowBranches(e.currentTarget.open)}>
        <summary>By military branch</summary>
        {showBranches && (
          <div className="fuel-detail-chart fh-frame">
            <FamilyWidget
              widget={branchChartWidget}
              options={branchChartOptions as unknown as FamilyViewProps['options']}
              mapOptions={EMPTY_MAP_OPTIONS}
              headerSlot={null}
              onDiagnostics={NOOP}
              runtime={runtime}
              active={active && showBranches}
              family="time-chart"
              declaration={fuelSeriesChartDeclaration}
              onActivate={handleChartActivate}
            />
          </div>
        )}
      </details>
    </>}
  </div>;
}

function DemandHistory({
  runtime,
  gate,
  widget,
  track,
  basis,
  keys,
  names,
  colorIndexes,
  hour,
  anchorHour,
  follows,
  playhead,
  extent,
  within,
  detailHours,
  active,
  onPick,
  label,
}: {
  readonly runtime: WidgetRuntime;
  readonly gate: Gate;
  readonly widget: WidgetBase;
  readonly track: BoundTrack;
  readonly basis: FuelBasis;
  readonly keys: readonly string[];
  readonly names: ReadonlyMap<string, string | null>;
  readonly colorIndexes: ReadonlyMap<string, number>;
  readonly hour: number | null;
  readonly anchorHour: number | null;
  readonly follows: boolean;
  readonly playhead: number;
  readonly extent: readonly [number, number];
  readonly within: readonly Span[] | null;
  readonly detailHours: number;
  readonly active: boolean;
  readonly onPick: (hour: number) => void;
  readonly label: string;
}) {
  const playing = usePlaybackMotion(runtime.clock, active && follows) !== undefined;
  const liveAnchor = usePlaybackWindowAnchor(runtime.clock, active && follows && playing, track.axisOffset, extent, detailHours);
  const anchor = follows && playing ? liveAnchor : (anchorHour ?? hour ?? basis.hour);
  const [start, end] = demandHistoryRange(extent, anchor, detailHours);
  const [visible, onViewChange] = useDetailView(start, end, detailHours);
  const wanted = useMemo(() => detailWindows(extent, visible, anchor, detailHours), [extent[0], extent[1], visible[0], visible[1], anchor, detailHours]);
  const basisAt = gate.recorded(basis.hour);
  const windows = useProductWindows<FuelDemandHistoryReply>(
    runtime.client,
    basisAt === null ? null : JSON.stringify([track.source.id, track.source.revision, basis, keys]),
    window => productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'country-fuel-demand-history',
      params: { branchId: basis.branchId, country: countryFromKey(basis.tag), hour: basisAt!, window, keys },
    }),
    wanted,
    active,
    validateFuelDemandHistory,
    track.source,
  );
  const series = useMemo(() => keys.map(key => ({
    id: key,
    name: names.get(key) ?? 'Name unavailable',
    colorIndex: colorIndexes.get(key) ?? 0,
    runs: joinedRuns([...windows.data.values()].map(reply => reply.parts.find(part => part.key === key)?.runs ?? [])),
  })), [keys, names, colorIndexes, windows.data]);

  const chartWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:fuel-demand-history:${keys.length === 1 ? keys[0] : 'all'}`,
    kind: FUEL_HISTORY_CHART_KIND,
    time: follows ? { mode: 'follow' } : pinnedTime(track, playhead),
  }), [widget, track, keys, follows, playhead]);

  const loading = active && basisAt !== null && wanted.some(window => !windows.data.has(window) && !windows.errors.has(window));
  const failed = windows.errors.size > 0;

  const chartOptions = useMemo<FuelHistoryChartOptions>(() => ({
    label,
    series,
    extent,
    viewX: [start, end],
    selectedHour: follows ? null : hour,
    within,
    minZero: true,
    ...(loading ? { pending: true } : {}),
    ...(failed ? { failed: true } : {}),
    surface: '',
  }), [label, series, extent, start, end, follows, hour, within, loading, failed]);

  const handleViewport = useCallback((vp: FamilyViewportChange) => {
    onViewChange(vp.window ?? null);
  }, [onViewChange]);

  const handleActivate = useCallback((action: InstalledAction) => {
    onPick(Math.floor(action.hour));
  }, [onPick]);

  return <>
    {loading && <p className="drill-empty">Loading consumption history…</p>}
    {[...windows.errors.values()].map((reason, i) => <WidgetState key={i} kind="error" what="consumption history" failure={reason} onRetry={windows.retry} />)}
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
