import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import type { WidgetProps } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { resolveWindow } from '../../dashboard/model';
import { useFollowedProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { clockRequest } from '../../dashboard/widgetRequests';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import { tableSettingsOf } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import FamilyTableHost, { type FamilySourceSubscription } from '../../families/table/FamilyTableHost.tsx';
import type { CampaignClockReply } from '../../generated/dashboard';
import { perfLog } from '../../shell/perf.ts';
import { clockHeadQuery } from '../player-apm/apmHead.ts';
import { validateClock } from './clockData';
import { CLOCK_COLUMNS, CLOCK_TABLE_BASE, campaignClockProjection } from './clockProjection.ts';

const OPTIONS_EMPTY: Record<string, never> = {};
const SETTINGS = tableSettingsOf(undefined, CLOCK_COLUMNS, CLOCK_TABLE_BASE);

export default function CampaignClockWidget({ widget, runtime, active, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const coverage = useRegionCoverage(runtime, useRegion(widget));

  const checked = useMemo(
    () => (following && active && track ? gate.checkFollow(clockHeadQuery(track.branch)) : null),
    [following, active, track?.branch.id, track?.branch.firstHour, track?.branch.lastHour, gate.checkFollow],
  );
  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(
    () => (track && checked?.query
      ? [{ name: 'main', source: track.source, branch: track.branch.id, offset: track.axisOffset, query: checked.query, channel: 'followed' }]
      : []),
    [track?.source, track?.branch.id, track?.axisOffset, checked?.query],
  );

  const hour = usePlaybackHour(runtime.clock, false);
  const window = track && resolveWindow(widget, track, runtime.tracks, hour);
  const request = following ? null : clockRequest(gate, widget, track, window ?? null);
  const { data, error, retry, moment } = useFollowedProduct<CampaignClockReply>(
    runtime.client,
    request,
    ['firstHour', 'lastHour'],
    active && !following,
    validateClock,
    track?.source,
  );
  const showing = data && !error && moment ? (JSON.parse(moment) as [number, number])[1] : null;
  const staticSources = useMemo<{ readonly sources: FamilySources; readonly hour: number } | null>(() => {
    if (following || !data) return null;
    return {
      sources: { answers: new Map(), panels: new Map([['main', data]]), live: new Map() },
      hour: showing ?? track?.branch.lastHour ?? 0,
    };
  }, [following, data, showing, track?.branch.lastHour]);

  const exactSources = useCallback(async (wantedHour: number): Promise<FamilySources | null> => {
    if (!track || !perfLog) return null;
    const recorded = gate.recorded(wantedHour);
    if (recorded === null) return null;
    const last = Math.min(recorded, track.branch.lastHour);
    const span = gate.span(track.branch.firstHour, last);
    if (!span) return null;
    const req = productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'campaign-clock',
      params: { branchId: track.branch.id, ...span },
    });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await runtime.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map() };
  }, [track, gate, runtime.client]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" icon="clock" message="Load a campaign to see its recorded clock times." />;
    if (coverage.outside) return <WidgetState kind="outside" read={coverage} />;
    if (following) {
      if (checked?.refused) return <WidgetState kind="unavailable" message={checked.refused} />;
      return null;
    }
    if (!request) return <WidgetState kind="outside" />;
    if (error) return <WidgetState kind="error" what="campaign clock" failure={error} onRetry={retry} />;
    if (!data) return <WidgetState kind="loading" label="Loading campaign clock…" />;
    return null;
  })();

  return <FamilyTableHost
    declaration={campaignClockProjection}
    options={OPTIONS_EMPTY}
    label="Recorded campaign clock times"
    subject="campaign clock"
    widget={widget}
    runtime={runtime}
    active={active}
    columns={CLOCK_COLUMNS}
    settings={SETTINGS}
    subscriptions={subscriptions}
    staticSources={staticSources}
    exactSources={track && perfLog ? exactSources : undefined}
    offset={track?.axisOffset ?? 0}
    pin={following ? null : staticSources?.hour ?? null}
    bounds={bounds}
    headerSlot={headerSlot}
    onDiagnostics={onDiagnostics}
    blockedState={blockedState}
    loadingLabel="Loading campaign clock…"
  />;
}
