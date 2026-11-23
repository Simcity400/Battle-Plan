import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { lookStarts } from '../../dashboard/countryTimeline';
import type { WidgetProps } from '../../dashboard/definitions';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { resolveOptions } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { forwardRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import type { TableSettings } from '../../draw/tables/logic.ts';
import { encodeTableSettings, TABLE_SETTINGS_KEY, tableSettingsOf } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import FamilyTableHost, { type FamilySourceSubscription } from '../../families/table/FamilyTableHost.tsx';
import { perfLog } from '../../shell/perf.ts';
import { COLUMNS, OPTIONS, TABLE_BASE } from './definition';
import {
  OCCUPIED_COLUMNS,
  occupiedStatesProjection,
  type OccupiedFilter,
  type OccupiedProjectionOptions,
} from './occupiedProjection.ts';
import { validateOccupiedStates, type CountryRole, type OccupiedStatesReply } from './occupiedStatesData.ts';
import './occupiedStates.css';

const NO_TAGS: readonly string[] = [];
const EVERY_COUNTRY: OccupiedFilter = { passing: null, unrecorded: [], bounds: [] };
const filters = new WeakMap<object, Map<string, OccupiedFilter>>();

function filterTimeline(
  tags: readonly string[],
  matches: (tag: string, hour: number) => boolean,
  unrecordedAt: (hour: number) => string | null,
  changes: readonly number[],
  first: number,
  last: number,
): OccupiedFilter {
  const bounds = [...new Set([first, ...changes.filter(at => at > first && at <= last), last + 1])].sort((a, b) => a - b);
  const passing: Record<string, [number, number][]> = {};
  const unrecorded: [number, number, string][] = [];
  for (let at = 0; at + 1 < bounds.length; at++) {
    const from = bounds[at]!, to = bounds[at + 1]! - 1;
    for (const tag of tags) if (matches(tag, from)) {
      const ranges = passing[tag] ??= [];
      const previous = ranges.at(-1);
      if (previous && previous[1] === from - 1) previous[1] = to;
      else ranges.push([from, to]);
    }
    const reason = unrecordedAt(from);
    const previous = unrecorded.at(-1);
    if (reason !== null) {
      if (previous && previous[1] === from - 1 && previous[2] === reason) previous[1] = to;
      else unrecorded.push([from, to, reason]);
    }
  }
  return { passing, unrecorded, bounds };
}

export default function OccupiedStatesWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const options = resolveOptions(OPTIONS, widget.options);
  const role: CountryRole = options.countryRole === 'occupied' ? 'occupied' : 'occupier';
  const gate = useGate(runtime, widget, SPEC, active);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const first = track?.branch.firstHour;
  const last = track?.branch.lastHour;
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const pinHour = !following && track ? gate.hourAt(axisAt(playbackHour)) : null;

  const groups = useCountryGroups(runtime, track, active);
  const tagsText = useMemo(() => (groups.data?.countryIdentities ?? []).map(history => history.tag).sort().join(','), [groups.data]);
  const tags = useMemo(() => (tagsText ? tagsText.split(',') : NO_TAGS), [tagsText]);
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(
    () => countryMatch(widget.countries, fallback, groups.data, last ?? 0),
    [widget.countries, fallback, groups.data, last],
  );
  const filter = useMemo<OccupiedFilter>(() => {
    if (widget.countries.kind === 'all' || !match.ready || !groups.data || first === undefined || last === undefined) return EVERY_COUNTRY;
    const key = JSON.stringify([widget.countries, fallback, first, last]);
    let held = filters.get(groups.data);
    if (!held) filters.set(groups.data, held = new Map());
    let made = held.get(key);
    if (!made) {
      made = filterTimeline(
        tags,
        match.matches,
        at => membershipUnrecordedAt(widget.countries, groups.data, at),
        countryMatchChangeHours(match.matches) ?? [],
        first,
        last,
      );
      held.set(key, made);
    }
    return made;
  }, [widget.countries, match, groups.data, fallback, tags, first, last]);

  const timeline = useHeldTimeline(runtime.client, track, useContentRevision(runtime.client, active), active);
  const startsText = JSON.stringify(tags.map(tag => [tag, lookStarts(timeline, tag, first ?? 0, last ?? 0)]));
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wanted = useMemo(() => starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => (held ? [{ hour: at, tag }] : []))), [starts]);
  const { exactAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Infinity, true);
  const labelsText = JSON.stringify(starts.map(([tag, hours]) => [tag, hours.map(([at, held]) => {
    const look = held ? exactAt(at)(tag) : undefined;
    return [at, look?.known ? look.name : null, look?.flag ?? null, look && !look.known ? look.name : null];
  })]));
  const labels = useMemo(
    () => Object.fromEntries((JSON.parse(labelsText) as [string, [number, string | null, string | null, string | null][]][])
      .map(([tag, runs]) => [tag, runs.map(([from, name, flag, status]) => ({ from, name, flag: flag ?? undefined, status: status ?? undefined }))])),
    [labelsText],
  );

  const looks = useCountryLooks(runtime.client, track, tags, last ?? 0, active);
  const countryColours = useMemo(() => new Map(tags.flatMap(tag => {
    const colour = looks.color(tag, last ?? 0);
    return colour ? [[tag, `rgb(${colour[0]}, ${colour[1]}, ${colour[2]})`] as const] : [];
  })), [tags, looks, last]);

  const settings = useMemo(() => tableSettingsOf(options.tableSettings, COLUMNS, TABLE_BASE), [options.tableSettings]);
  const onSettings = useCallback((next: TableSettings) => setOption?.(TABLE_SETTINGS_KEY, encodeTableSettings(next)), [setOption]);
  const lawAs = settings.columns.law?.as ?? 'dot';

  const projectionOptions = useMemo<OccupiedProjectionOptions>(() => ({
    filter,
    role,
    labels,
    lawAs,
    filterAll: widget.countries.kind === 'all',
  }), [filter, role, labels, lawAs, widget.countries.kind]);

  const checked = useMemo(
    () => (following && active && track ? gate.checkFollow({ panel: 'occupied-states', branchId: track.branch.id }) : null),
    [following, active, track?.branch.id, gate.checkFollow],
  );
  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(
    () => (track && checked?.query
      ? [{ name: 'main', source: track.source, branch: track.branch.id, offset: track.axisOffset, query: checked.query, channel: 'panel', product: 'occupied-states' }]
      : []),
    [track?.source, track?.branch.id, track?.axisOffset, checked?.query],
  );

  const pinnedReq = useMemo(
    () => (!following && track && pinHour !== null ? forwardRequest(track.source, 'occupied-states', { branchId: track.branch.id, hour: pinHour }) : null),
    [following, track, pinHour],
  );
  const pinned = useFollowedProduct<OccupiedStatesReply>(runtime.client, pinnedReq, [], active && !following, validateOccupiedStates, track?.source);
  const staticSources = useMemo<{ readonly sources: FamilySources; readonly hour: number } | null>(() => {
    if (following || !pinned.data || pinHour === null) return null;
    return {
      sources: { answers: new Map(), panels: new Map([['main', pinned.data]]), live: new Map([['main', repliedCells(pinned.data).read()]]) },
      hour: pinHour,
    };
  }, [following, pinned.data, pinHour]);

  const exactSources = useCallback(async (wantedHour: number): Promise<FamilySources | null> => {
    if (!track || !perfLog) return null;
    const recorded = gate.recorded(wantedHour);
    if (recorded === null) return null;
    const req = forwardRequest(track.source, 'occupied-states', { branchId: track.branch.id, hour: recorded });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await runtime.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map([['main', repliedCells(raw).read()]]) };
  }, [track, gate, runtime.client]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (first !== undefined && last !== undefined ? [first, last] : null),
    [first, last],
  );

  const contextMenuItems = useCallback(
    (_key: string, _name: string, atHour: number) => (track ? [jumpItem(runtime, track.id, atHour)] : []),
    [runtime, track],
  );

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" message="Load a campaign to see occupied states." />;
    if (coverage.outside) return <WidgetState kind="outside" read={coverage} />;
    if (groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
    if (!match.ready || !groups.data) return <WidgetState kind="loading" label="Loading occupied states…" />;
    if (following) {
      if (checked?.refused) return <WidgetState kind="unavailable" message={checked.refused} />;
      return null;
    }
    if (pinned.error && !pinned.data) return <WidgetState kind="error" failure={pinned.error} onRetry={pinned.retry} />;
    if (pinHour === null) return <WidgetState kind="outside" />;
    if (!pinned.data) return <WidgetState kind="loading" label="Loading occupied states…" />;
    return null;
  })();

  return <div className="occupied-states">
    {groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    <FamilyTableHost
      declaration={occupiedStatesProjection}
      options={projectionOptions}
      label="Occupied states"
      subject="occupied states"
      className="occupied-table-surface"
      widget={widget}
      runtime={runtime}
      active={active}
      columns={OCCUPIED_COLUMNS}
      settings={settings}
      onSettings={setOption ? onSettings : undefined}
      subscriptions={subscriptions}
      staticSources={staticSources}
      exactSources={track && perfLog && match.ready && !match.reason ? exactSources : undefined}
      offset={track?.axisOffset ?? 0}
      pin={following ? null : pinHour}
      bounds={bounds}
      countryColours={countryColours}
      contextMenuItems={contextMenuItems}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      blockedState={blockedState}
      loadingLabel="Loading occupied states…"
    />
  </div>;
}
