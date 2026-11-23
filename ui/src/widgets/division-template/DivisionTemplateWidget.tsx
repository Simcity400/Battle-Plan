import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { formatDateTime } from '../../dashboard/calendar';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { WidgetProps } from '../../dashboard/definitions';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import GateState from '../../dashboard/GateState';
import { resolveOptions } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime.ts';
import { forwardRequest } from '../../dashboard/request.ts';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';
import type { TableSettings } from '../../draw/tables/logic.ts';
import { tableSettingsOf } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import FamilyTableHost, { type FamilySourceSubscription } from '../../families/table/FamilyTableHost.tsx';
import type { DivisionTemplatesReply } from '../../generated/dashboard';
import { perfLog } from '../../shell/perf.ts';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { designerSnapshot, subscribeDesignerSnapshot } from './designerSnapshot';
import { validateDivisionTemplates } from './divisionTemplateData';
import {
  divisionTemplateColumns,
  divisionTemplateProjection,
  type DivisionTemplateProjectionOptions,
} from './divisionTemplateProjection.ts';
import './divisionTemplate.css';

const NO_LINES: readonly string[] = [];

export default function DivisionTemplateWidget({ widget, runtime, active, setOption, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillCoverage = useRegionCoverage(runtime, drillRegion);
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const gate = useGate(runtime, widget, SPEC, active, { country: { stored: widget.options?.country, setOption } });
  const country = gate.country!;
  const hour = gate.hour;
  const tag = gate.open ? country.tag : null;
  const countryLabel = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const countryName = countryLabel.name;
  const nameDetail = countryLabel.nameFailure ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span> : undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const countFor = options.countFor === 'host' ? 'host' : 'owner';

  const snapshot = useSyncExternalStore(subscribeDesignerSnapshot, designerSnapshot, designerSnapshot);
  const anchor = useRef<HTMLDivElement>(null);
  const [opened, setOpened] = useState<{ template: string; designer: 'ordinary' | 'headquarters'; tag: string } | null>(null);
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_LINES);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(previous => (previous.length === lines.length && previous.every((line, index) => line === lines[index]) ? previous : lines)), []);
  useEffect(() => {
    dispatch({ type: 'close' });
    setOpened(null);
  }, [track?.source.id, track?.source.revision, track?.branch.id, country.chosen, dispatch]);

  const checked = useMemo(
    () => (following && active && track && tag ? gate.checkFollow({ panel: 'division-templates', branchId: track.branch.id, country: countryFromKey(tag), countFor }) : null),
    [following, active, track?.branch.id, tag, countFor, gate.checkFollow],
  );
  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(
    () => (track && checked?.query
      ? [{ name: 'main', source: track.source, branch: track.branch.id, offset: track.axisOffset, query: checked.query, channel: 'panel', product: 'division-templates' }]
      : []),
    [track?.source, track?.branch.id, track?.axisOffset, checked?.query],
  );

  const playbackHour = usePlaybackHour(runtime.clock, false);
  const pinHour = !following && track && tag ? gate.hourAt(axisAt(playbackHour)) : null;
  const pinnedReq = useMemo(
    () => (!following && track && tag && pinHour !== null
      ? forwardRequest(track.source, 'division-templates', { branchId: track.branch.id, hour: pinHour, country: countryFromKey(tag), countFor })
      : null),
    [following, track, tag, pinHour, countFor],
  );
  const pinned = useFollowedProduct<DivisionTemplatesReply>(runtime.client, pinnedReq, [], active && !following, validateDivisionTemplates, track?.source);
  const staticSources = useMemo<{ readonly sources: FamilySources; readonly hour: number } | null>(() => {
    if (following || !pinned.data || pinHour === null) return null;
    return {
      sources: { answers: new Map(), panels: new Map([['main', pinned.data]]), live: new Map([['main', repliedCells(pinned.data).read()]]) },
      hour: pinHour,
    };
  }, [following, pinned.data, pinHour]);

  const exactSources = useCallback(async (wantedHour: number): Promise<FamilySources | null> => {
    if (!track || !tag || !perfLog) return null;
    const recorded = gate.recorded(wantedHour);
    if (recorded === null) return null;
    const req = forwardRequest(track.source, 'division-templates', { branchId: track.branch.id, hour: recorded, country: countryFromKey(tag), countFor });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await runtime.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map([['main', repliedCells(raw).read()]]) };
  }, [track, tag, countFor, gate, runtime.client]);

  const columns = useMemo(() => divisionTemplateColumns(options.cellBars === true), [options.cellBars]);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const overflow = narrowTableOf(options) === 'scroll' ? 'scroll' as const : 'tooltip' as const;
  const density = options.density === 'compact' ? 'compact' as const : 'comfortable' as const;
  const settings = useMemo<TableSettings>(
    () => tableSettingsOf(undefined, columns, { density, overflow, sort }),
    [columns, density, overflow, sort.column, sort.direction],
  );
  const onSettings = useCallback((next: TableSettings) => {
    if (next.sort.column !== sort.column || next.sort.direction !== sort.direction) {
      setOption?.('tableSort', tableSortToken(next.sort));
    }
  }, [sort.column, sort.direction, setOption]);

  const projectionOptions = useMemo<DivisionTemplateProjectionOptions>(() => ({
    countryName,
    cellBars: options.cellBars === true,
    canOpenDesigner: Boolean(snapshot),
  }), [countryName, options.cellBars, snapshot]);

  const handleActivate = useCallback((key: string, meta?: unknown, atHour?: number) => {
    if (!snapshot || !tag) return;
    const info = meta as { designer?: 'ordinary' | 'headquarters'; tag?: string } | null | undefined;
    const at = atHour ?? pinHour ?? hour;
    if (at === null) return;
    setOpened({ template: key, designer: info?.designer ?? 'ordinary', tag: info?.tag ?? tag });
    drillControl.show(key, at);
  }, [snapshot, tag, pinHour, hour, drillControl]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" message="Load a campaign to see templates." />;
    if (!gate.open) {
      return <GateState gate={gate} loading="Loading templates…" choose="Choose a country to see its templates."
        absent={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
    }
    if (following) {
      if (checked?.refused) return <WidgetState kind="unavailable" message={checked.refused} />;
      return null;
    }
    if (pinned.error && !pinned.data) return <WidgetState kind="error" failure={pinned.error} onRetry={pinned.retry} />;
    if (pinHour === null) return <WidgetState kind="outside" />;
    if (!pinned.data) return <WidgetState kind="loading" label="Loading templates…" />;
    return null;
  })();

  const main = <FamilyTableHost
    declaration={divisionTemplateProjection}
    options={projectionOptions}
    label="Division templates"
    subject="templates"
    widget={widget}
    runtime={runtime}
    active={active}
    columns={columns}
    settings={settings}
    onSettings={setOption ? onSettings : undefined}
    subscriptions={subscriptions}
    staticSources={staticSources}
    exactSources={track && tag && perfLog ? exactSources : undefined}
    offset={track?.axisOffset ?? 0}
    pin={following ? null : pinHour}
    bounds={bounds}
    selectedKey={opened?.template ?? null}
    onActivate={snapshot ? handleActivate : undefined}
    headerSlot={headerSlot}
    onDiagnostics={onDiagnostics}
    extraWarnings={windowLines}
    blockedState={blockedState}
    loadingLabel="Loading templates…"
  />;

  const draw = snapshot;
  return <div ref={anchor} className="division-template" data-detail={countryLabel.nameFailure?.message}>
    {country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" failure={country.error} onRetry={country.retry} /> : null}
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && !!draw && !!opened} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title="Division designer" day={drillAt === null ? '' : formatDateTime(drillAt)} canFollow anchor={anchor}
      onClose={() => { dispatch({ type: 'close' }); setOpened(null); setWindowLines(NO_LINES); }} main={main}>
      {draw && opened && track && drillAt === null && <WidgetState kind="outside" read={drillCoverage} />}
      {draw && opened && track && drillAt !== null && draw({ client: runtime.client, source: track.source, branchId: track.branch.id, hour: drillAt,
        tag: opened.tag, template: opened.template, designer: opened.designer, onDiagnostics: onWindowDiagnostics }, active)}
    </DrillHost>
  </div>;
}
