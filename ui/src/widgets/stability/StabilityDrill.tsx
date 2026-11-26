import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Segmented } from '../../components/Segmented';
import WidgetState from '../../components/WidgetState';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { FamilyViewProps } from '../../dashboard/definitions';
import { heldParts, validateSeriesParts, type HeldParts, type SeriesParts } from '../../dashboard/keyedSeriesData.ts';
import type { BoundTrack, WidgetBase, WidgetTime } from '../../dashboard/model';
import { loadTransientProduct, useProducts } from '../../dashboard/products';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import type { Gate } from '../../dashboard/regionGate';
import type { Region } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { TimeRegion } from '../../dashboard/widgetTime.ts';
import type { FamilyViewportChange } from '../../families/host/adapter.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { measureOf, namesAt, validateStabilityPartNames } from './partsData.ts';
import type { Measure } from './stabilityData';
import {
  buildStabilityNextChangeParams,
  MEASURE_NAMES,
  PARTS_FAMILY,
  STABILITY_PARTS_CHART_KIND,
  STABILITY_PARTS_TABLE_KIND,
  stabilityPartsChartDeclaration,
  stabilityPartsTableDeclaration,
  type StabilityDetailOptions,
  type StabilityNavKey,
  type StabilitySeriesOwner,
} from './stabilityDetailProjection.ts';
import type { StabilityPartNamesReply } from './types';

const EMPTY_MAP_OPTIONS: FamilyViewProps['mapOptions'] = { country: null, fog: false };
const NOOP = () => {};

function isStabilityNavKey(key: string): key is StabilityNavKey {
  return key === 'ArrowLeft' || key === 'ArrowRight' || key === 'Home' || key === 'End';
}

function pinnedTime(track: BoundTrack, hour: number): WidgetTime {
  const offset = Math.floor(hour) - track.branch.firstHour;
  return { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } };
}

function rangeTime(track: BoundTrack, firstHour: number, lastHour: number): WidgetTime {
  const firstOffset = Math.floor(firstHour) - track.branch.firstHour;
  const lastOffset = Math.max(firstOffset, Math.floor(lastHour) - track.branch.firstHour);
  return { mode: 'range', firstOffset, lastOffset };
}

export default function StabilityDrill({
  widget,
  runtime,
  active,
  gate,
  region,
  track,
  owner,
  measure,
  onMeasure,
  hour,
  following,
  playhead,
  extent,
  windowHours,
  onPick,
  onDiagnostics,
}: FamilyViewProps & {
  readonly gate: Gate;
  readonly region: Region;
  readonly track: BoundTrack;
  readonly owner: StabilitySeriesOwner | null;
  readonly measure: Measure;
  readonly onMeasure?: (measure: Measure) => void;
  readonly hour: number | null;
  readonly following: boolean;
  readonly playhead: number | null;
  readonly extent: readonly [number, number];
  readonly windowHours?: number | null;
  readonly onPick?: (hour: number) => void;
}) {
  const ownerKey = owner?.key ?? null;
  const span = useMemo(() => gate.span(extent[0], extent[1]), [extent[0], extent[1], gate.span]);
  const windows = useMemo(() => {
    if (!owner || !span) return [];
    if (owner.spans === null) return [span];
    return owner.spans.flatMap(held => gate.span(Math.max(held.firstHour, span.firstHour), Math.min(held.lastHour, span.lastHour)) ?? []);
  }, [owner, span?.firstHour, span?.lastHour, gate.span]);
  const partsRequests = useMemo(() => (
    ownerKey
      ? windows.map(window => productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'series-parts',
          params: { branchId: track.branch.id, family: PARTS_FAMILY, countries: [countryFromKey(ownerKey)], from: window.firstHour, to: window.lastHour },
        }))
      : []
  ), [track.source.id, track.source.revision, track.branch.id, ownerKey, windows]);
  const validateParts = useCallback((value: unknown) => validateSeriesParts(value, ownerKey ?? ''), [ownerKey]);
  const listedRead = useProducts<SeriesParts>(runtime.client, partsRequests, active && partsRequests.length > 0, validateParts, track.source);
  const listed = useMemo<HeldParts | undefined>(() => {
    if (ownerKey === null || !owner || !span) return undefined;
    if (!partsRequests.length) return { tag: ownerKey, parts: [], missing: [], partBits: 1 };
    if (!listedRead.data) return undefined;
    return {
      tag: ownerKey,
      partBits: listedRead.data[0]?.partBits ?? 1,
      parts: heldParts(listedRead.data.flatMap((reply, at) => reply.parts.flatMap(part => {
        const first = Math.max(part.firstHour, windows[at]!.firstHour);
        const last = Math.min(part.lastHour, windows[at]!.lastHour);
        return first <= last ? [{ ...part, firstHour: first, lastHour: last }] : [];
      }))),
      missing: listedRead.data.flatMap(reply => reply.missing),
    };
  }, [ownerKey, owner, span, partsRequests.length, listedRead.data, windows]);

  const allParts = useMemo(() => (listed?.parts ?? []).map(part => part.part), [listed]);
  const namesRequests = useMemo(() => (
    ownerKey && span && listed
      ? namesAt(listed.parts, span.lastHour).flatMap(([at, parts]) => {
          const recorded = gate.recorded(at);
          return recorded === null ? [] : [productRequest({
            sourceId: track.source.id,
            revision: track.source.revision,
            product: 'stability-part-names',
            params: { branchId: track.branch.id, country: countryFromKey(ownerKey), parts, hour: recorded },
          })];
        })
      : []
  ), [track.source.id, track.source.revision, track.branch.id, ownerKey, listed, span, gate.recorded]);
  const namedRead = useProducts<StabilityPartNamesReply>(runtime.client, namesRequests, active && namesRequests.length > 0, validateStabilityPartNames, track.source);
  const names = useMemo(() => namedRead.data?.flatMap(reply => reply.parts) ?? [], [namedRead.data]);
  const warnings = useMemo(() => [...new Set(namedRead.data?.flatMap(reply => reply.warnings) ?? [])], [namedRead.data]);
  const parts = useMemo(() => allParts.filter(part => measureOf(part) === measure), [allParts, measure]);
  const namesReady = !listed || namesRequests.length === 0 || namedRead.data !== undefined;
  const missingReasons = useMemo(() => {
    const raw = listed?.missing ?? [];
    if (!raw.length) return [];
    const lo = extent[0];
    const hi = following ? extent[1] : Math.min(extent[1], Math.floor(playhead ?? extent[1]));
    const reasons: string[] = [];
    for (const item of raw) {
      if (item.firstHour <= hi && item.lastHour >= lo && item.reason) reasons.push(item.reason);
    }
    return [...new Set(reasons)];
  }, [listed?.missing, extent[0], extent[1], following, playhead]);

  const day = hour == null ? null : Math.floor(hour);
  const visibleWindowRef = useRef<readonly [number, number] | null>(null);
  const navScope = JSON.stringify([track.source.id, track.source.revision, track.branch.id, ownerKey, measure, parts]);
  const navScopeRef = useRef(navScope);
  navScopeRef.current = navScope;
  const navAbortRef = useRef<AbortController | null>(null);
  const [navError, setNavError] = useState<{ readonly scope: string; readonly failure: Failure } | null>(null);
  useEffect(() => {
    visibleWindowRef.current = null;
  }, [track.source.id, track.source.revision, track.branch.id, ownerKey]);
  useEffect(() => {
    navAbortRef.current?.abort();
    navAbortRef.current = null;
    return () => {
      navAbortRef.current?.abort();
      navAbortRef.current = null;
    };
  }, [navScope, active]);

  const handleViewportChange = useCallback((vp: FamilyViewportChange) => {
    visibleWindowRef.current = vp.window ?? null;
  }, []);

  const handleChartKeyDownCapture = useCallback((event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (!isStabilityNavKey(event.key)) return;
    if (!active || !owner || !parts.length || !onPick) return;
    const livePlayhead = following ? Math.floor(runtime.clock.getHour()) + track.axisOffset : playhead;
    const visible: readonly [number, number] = visibleWindowRef.current
      ?? (following && windowHours != null && livePlayhead !== null
        ? [Math.max(extent[0], livePlayhead - windowHours), Math.min(extent[1], livePlayhead)]
        : extent);
    const params = buildStabilityNextChangeParams({
      branchId: track.branch.id,
      owner,
      parts,
      key: event.key,
      fromHour: following ? livePlayhead : day,
      visible,
      playhead: livePlayhead,
      gate,
    });
    if (!params) return;
    event.preventDefault();
    event.stopPropagation();
    navAbortRef.current?.abort();
    const controller = new AbortController();
    navAbortRef.current = controller;
    const scope = navScopeRef.current;
    const req = productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'series-next-change',
      params,
    });
    void loadTransientProduct<{ hour: unknown }>(runtime.client, req, controller.signal).then(reply => {
      if (controller.signal.aborted || navScopeRef.current !== scope) return;
      if (reply.hour !== null && !Number.isSafeInteger(reply.hour)) {
        throw new Error('The keyboard reply is malformed.');
      }
      setNavError(null);
      if (typeof reply.hour === 'number') onPick(reply.hour);
    }).catch(err => {
      if (!controller.signal.aborted && navScopeRef.current === scope) {
        setNavError({ scope, failure: asFailure(err) });
      }
    });
  }, [active, owner, parts, onPick, track.source.id, track.source.revision, track.branch.id, track.axisOffset, following, playhead, day, extent, windowHours, gate, runtime.clock, runtime.client]);

  const tableWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:stability-parts-table`,
    kind: STABILITY_PARTS_TABLE_KIND,
    time: following ? { mode: 'follow' } : pinnedTime(track, day ?? extent[1]),
  }), [widget, track, following, day, extent[1]]);
  const chartWidget = useMemo<WidgetBase>(() => ({
    ...widget,
    id: `${widget.id}:stability-parts-chart`,
    kind: STABILITY_PARTS_CHART_KIND,
    time: following ? { mode: 'follow' } : rangeTime(track, extent[0], Math.min(extent[1], Math.floor(playhead ?? extent[1]))),
  }), [widget, track, following, extent[0], extent[1], playhead]);

  const sharedOptions = useMemo<StabilityDetailOptions>(() => ({
    ownerKey,
    spans: owner?.spans ?? null,
    parts,
    partBits: listed?.partBits ?? 1,
    names,
    namesReady,
    warnings,
    ...(missingReasons.length ? { missingReasons } : {}),
    measure,
    extent,
    selectedHour: following ? null : day,
    ...(windowHours != null ? { windowHours } : {}),
    surface: '',
  }), [ownerKey, owner?.spans, parts, listed?.partBits, names, namesReady, warnings, missingReasons, measure, extent, following, day, windowHours]);

  const activeNavError = navError?.scope === navScope ? navError.failure : null;
  const error = listedRead.error ?? namedRead.error ?? activeNavError;
  const retry = () => {
    setNavError(null);
    listedRead.retry();
    namedRead.retry();
  };

  return <div className="stability-drill">
    {onMeasure && <Segmented label="Breakdown of" value={measure} onChange={onMeasure}
      options={(['stability', 'warSupport'] as const).map(value => ({ value, label: MEASURE_NAMES[value] }))} />}
    {error && <WidgetState kind="error" failure={error} onRetry={retry} />}
    {!owner ? (
      <WidgetState kind="unavailable" message="Selected country unavailable." />
    ) : !listed || !namesReady ? (
      error ? null : <WidgetState kind="loading" label="Loading the breakdown…" />
    ) : (
      <>
        <TimeRegion runtime={runtime} region={region}>
          <div className="stability-drill-table fh-frame">
            <FamilyWidget
              widget={tableWidget}
              options={sharedOptions as unknown as FamilyViewProps['options']}
              mapOptions={EMPTY_MAP_OPTIONS}
              headerSlot={null}
              runtime={runtime}
              active={active}
              onDiagnostics={onDiagnostics}
              family="table"
              declaration={stabilityPartsTableDeclaration}
            />
          </div>
        </TimeRegion>
        <div className="stability-drill-chart fh-frame" onKeyDownCapture={handleChartKeyDownCapture}>
          <FamilyWidget
            widget={chartWidget}
            options={sharedOptions as unknown as FamilyViewProps['options']}
            mapOptions={EMPTY_MAP_OPTIONS}
            headerSlot={null}
            onDiagnostics={NOOP}
            runtime={runtime}
            active={active}
            family="time-chart"
            declaration={stabilityPartsChartDeclaration}
            onViewportChange={handleViewportChange}
            onActivate={onPick ? action => onPick(Math.floor(action.hour)) : undefined}
          />
        </div>
      </>
    )}
  </div>;
}
