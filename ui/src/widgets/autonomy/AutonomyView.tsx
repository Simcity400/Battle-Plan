import { useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { countryFromKey } from '../../dashboard/countryKey';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { worldFilterQuery } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useTimelineIdentity } from '../../families/timeline/useTimelineIdentity';
import FamilyTimelineHost from '../../families/timeline/FamilyTimelineHost';
import type { SubjectReply } from '../../generated/dashboard';
import { useMeasureSeries } from '../country-measure/measureSeries';
import { validateMeasureStates, type MeasureStates } from '../country-measure/measureSeriesData';
import { validateSubjectReply } from '../subjects/subjectsData';
import { autonomyLanes } from './autonomyData';
import { autonomyProjection, type AutonomyProjectionOptions, type AutonomySeriesSample, type AutonomyViewMode } from './autonomyProjection';
import { OPTIONS } from './definition';

const NO_SUBS = [] as const;

export default function AutonomyView({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view: AutonomyViewMode = options.view === 'history' ? 'history' : 'meters';
  const bySubject = options.countriesAre === 'subjects';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const groups = useCountryGroups(runtime, track, active);
  const lastHour = track?.branch.lastHour ?? 0;
  const fallback = defaultCountryKey(track);

  const req = useMemo(
    () => (track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'subjects', params: { branchId: track.branch.id } }) : null),
    [track],
  );
  const product = useProduct<SubjectReply>(runtime.client, req, active, validateSubjectReply, track?.source);
  const data = product.data;

  const selection = useMemo(
    () => worldFilterQuery(widget.countries, fallback ? [fallback] : [], groups.data, lastHour),
    [widget.countries, fallback, groups.data, lastHour],
  );
  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const head = Math.floor(window?.lastHour ?? lastHour);
  const filter = useMemo(() => selection.at(head), [selection, head]);
  const lanesData = useMemo(() => (data ? autonomyLanes(data, filter.tags, bySubject) : []), [data, filter.tags, bySubject]);
  const tags = useMemo(() => lanesData.map(l => l.tag), [lanesData]);
  const within = useMemo(() => new Map(lanesData.map(l => [l.tag, l.within])), [lanesData]);

  const series = useMeasureSeries(
    runtime.client,
    gate,
    track?.source,
    track?.branch,
    'autonomy',
    'progress',
    tags,
    following ? null : gate.recorded(head),
    null,
    active && tags.length > 0,
    false,
    within,
    lastHour,
    following,
  );

  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const hovered = gate.recorded(hoverHour);
  const scoreReq = (measure: string) => (track && hovered !== null && tags.length > 0
    ? productRequest({
        sourceId: track.source.id,
        revision: track.source.revision,
        product: 'series-state',
        params: {
          branchId: track.branch.id,
          family: { kind: 'autonomy', measure },
          reads: tags.slice(0, 32).map(tag => ({ country: countryFromKey(tag), hour: hovered })),
        },
      })
    : null);
  const scores = useProduct<MeasureStates>(runtime.client, scoreReq('score'), active && hovered !== null, validateMeasureStates, track?.source);
  const lowers = useProduct<MeasureStates>(runtime.client, scoreReq('lower'), active && hovered !== null, validateMeasureStates, track?.source);
  const uppers = useProduct<MeasureStates>(runtime.client, scoreReq('upper'), active && hovered !== null, validateMeasureStates, track?.source);

  const hoverPanels = useMemo(() => {
    if (hovered === null) return undefined;
    const out: Record<string, { readonly hour: number; readonly data: unknown }> = {};
    if (scores.data) out.score = { hour: hovered, data: scores.data };
    if (lowers.data) out.lower = { hour: hovered, data: lowers.data };
    if (uppers.data) out.upper = { hour: hovered, data: uppers.data };
    return Object.keys(out).length ? out : undefined;
  }, [hovered, scores.data, lowers.data, uppers.data]);

  const seriesSamples = useMemo(() => {
    const out: Record<string, AutonomySeriesSample> = {};
    for (const tag of tags) {
      const summary = series.lineOf(tag);
      const pts: [number, number | null][] = [];
      let current: number | null = null;
      if (summary) {
        for (const b of summary.buckets) {
          if (!b) continue;
          const v = Number.isFinite(b.lastValue) ? Math.max(0, Math.min(1, b.lastValue)) : null;
          pts.push([b.lastHour, v]);
          if (v !== null) current = v;
        }
      }
      const line = series.lines.find(l => l.tag === tag);
      if (line && line.runs.length > 0 && pts.length === 0) {
        for (const r of line.runs) {
          const val = Number(r.valueRaw) / 100_000;
          const clamped = Number.isFinite(val) ? Math.max(0, Math.min(1, val)) : null;
          pts.push([r.firstHour, clamped]);
          if (r.lastHour > r.firstHour) pts.push([r.lastHour, clamped]);
          if (clamped !== null) current = clamped;
        }
      }
      out[tag] = { tag, points: pts, current };
    }
    return out;
  }, [tags, series.lineOf, series.lines, series.held]);

  const { snapshot: identitySnapshot, onDemands: onIdentityDemands } = useTimelineIdentity(runtime.client, track, active);

  const projectionOptions = useMemo<AutonomyProjectionOptions>(() => ({
    view,
    bySubject,
    selectedTags: filter.tags ? [...filter.tags] : null,
    branchFirst: track?.branch.firstHour ?? 0,
    surface: '#16181d',
    seriesSamples,
  }), [view, bySubject, filter.tags, track?.branch.firstHour, seriesSamples]);

  const staticSources = useMemo(() => (data
    ? { panels: { main: data }, hour: head, provenInterval: following && track ? [track.branch.firstHour, track.branch.lastHour] as const : null }
    : null), [data, head, following, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track],
  );

  const blocked = useMemo(() => {
    if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see its subjects’ autonomy." />;
    if (widget.time.mode !== 'follow' && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (groups.error && widget.countries.kind === 'group' && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (widget.countries.kind === 'group' && !groups.data) return <WidgetState kind="loading" label="Loading country membership…" />;
    if (filter.reason) return <WidgetState kind="empty" message={filter.reason} />;
    if (product.error) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
    if (series.error) return <WidgetState kind="error" failure={series.error} onRetry={series.retry} />;
    if (!data) return <WidgetState kind="loading" label="Loading autonomy…" />;
    return null;
  }, [track, widget.time.mode, window, groups.error, groups.data, groups.retry, widget.countries.kind, filter.reason, product.error, product.retry, series.error, series.retry, data]);

  return (
    <FamilyTimelineHost
      declaration={autonomyProjection}
      options={projectionOptions}
      label="Autonomy"
      subject="autonomy"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={NO_SUBS}
      allowEmptyFollow
      owner={track ? `autonomy:${track.source.id}:${track.branch.id}:${view}` : 'autonomy'}
      staticSources={staticSources}
      hoverPanels={hoverPanels}
      offset={track?.axisOffset ?? 0}
      pin={following ? null : head}
      bounds={bounds}
      identitySnapshot={identitySnapshot}
      onIdentityDemands={onIdentityDemands}
      onHoverHour={setHoverHour}
      contextMenuItems={track ? (_key, _name, h) => [jumpItem(runtime, track.id, h)] : undefined}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
    />
  );
}
