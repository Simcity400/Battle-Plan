import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { jumpItem } from '../../charts/jump';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { countryTagAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { lookStarts } from '../../dashboard/countryTimeline';
import type { WidgetProps } from '../../dashboard/definitions';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { resolveOptions } from '../../dashboard/options';
import { useFollowedProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { forwardRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import type { TableSettings } from '../../draw/tables/logic.ts';
import { tableSettingsOf } from '../../draw/tables/settings.ts';
import type { FamilySources } from '../../families/contract.ts';
import FamilyTableHost, { type FamilySourceSubscription } from '../../families/table/FamilyTableHost.tsx';
import { perfLog } from '../../shell/perf.ts';
import type { HourRanges } from '../occupied-states/occupiedProjection.ts';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { SCORE_LABELS, scoreCountries, scorePart, scoreParts, validateGameScore, type GameScoreReply } from './gameScoreData.ts';
import { gameScoreColumns, gameScoreProjection, type FactionRun, type GameScoreProjectionOptions } from './gameScoreProjection.ts';

const NO_WARNINGS: string[] = [];
const format = (raw: string, decimals: number) => formatRaw(raw, decimals);
const formatValue = (value: number) => Math.round(value).toLocaleString('en-US');
const SCORE_SPLIT = scoreParts();

export default function GameScoreWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const by = scorePart(options.topMeasure);
  const view = options.view === 'race' ? 'race' : options.view === 'lines' ? 'lines' : 'table';
  const measure = useMemo(() => ({ kind: 'country-game-scores', measure: by }), [by]);
  const onWarnings = useCallback((warnings: readonly string[]) => props.onDiagnostics(warnings.length ? [...warnings] : NO_WARNINGS), [props.onDiagnostics]);
  if (view === 'table') return <ScoreTable {...props} />;
  return <MeasureRace widget={props.widget} runtime={props.runtime} active={props.active} measure={measure} decimals={5} view={view} options={options}
    label={SCORE_LABELS[by]} icon="grid" format={format} formatValue={formatValue} onWarnings={onWarnings}
    parts={by === 'total' ? SCORE_SPLIT : undefined} split={view === 'race' && options.split !== false} breakdown={false} />;
}

function ScoreTable({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const by = scorePart(options.topMeasure);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const firstHour = track?.branch.firstHour ?? 0;
  const lastHour = track?.branch.lastHour ?? 0;
  const all = widget.countries.kind === 'all';
  const needsGroups = options.factions === true || (widget.countries.kind === 'group' && widget.countries.group.type !== 'top');
  const groups = useCountryGroups(runtime, track, active);
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const resolved = useMemo(
    () => scoreCountries(widget.countries, JSON.parse(fallbackKey) as string[], groups.data, lastHour),
    [widget.countries, fallbackKey, groups.data, lastHour],
  );
  const bySlot = resolved.tags.some(isSlotKey);
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const pinHour = !following && track ? gate.hourAt(axisAt(playbackHour)) : null;

  const chosenTags = useMemo(
    () => (bySlot ? resolved.tags.flatMap(key => countryTagAt(slots, countryFromKey(key), pinHour ?? lastHour) ?? []) : resolved.tags),
    [bySlot, resolved.tags, slots, pinHour, lastHour],
  );
  const membership = useMemo<Readonly<Record<string, HourRanges>> | null>(() => {
    if (!resolved.membership) return null;
    const out: Record<string, [number, number][]> = {};
    for (const [tag, runs] of resolved.membership) out[tag] = runs.map(run => [run.firstHour, run.lastHour]);
    return out;
  }, [resolved.membership]);

  const factionRuns = useMemo<readonly FactionRun[] | null>(() => {
    if (!groups.data) return null;
    return groups.data.factions.flatMap(faction => faction.members.map(run => ({
      id: faction.id,
      tag: run.tag,
      firstHour: run.firstHour,
      lastHour: run.lastHour,
      names: faction.names,
      fallbackName: faction.name,
    })));
  }, [groups.data]);

  const allTagsText = useMemo(() => JSON.stringify((groups.data?.countryIdentities ?? []).map(h => h.tag).sort()), [groups.data]);
  const allTags = useMemo(() => JSON.parse(allTagsText) as string[], [allTagsText]);
  const timeline = useHeldTimeline(runtime.client, track, useContentRevision(runtime.client, active), active);
  const startsText = JSON.stringify(allTags.map(tag => [tag, lookStarts(timeline, tag, firstHour, lastHour)]));
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wanted = useMemo(() => starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => (held ? [{ tag, hour: at }] : []))), [starts]);
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

  const columns = useMemo(() => gameScoreColumns(undefined, options.cellBars === true), [options.cellBars]);
  const overflow = narrowTableOf(options) === 'scroll' ? 'scroll' as const : 'tooltip' as const;
  const density = options.density === 'compact' ? 'compact' as const : 'comfortable' as const;
  const tableSettings = useMemo<TableSettings>(
    () => tableSettingsOf(undefined, columns, { density, overflow, sort }),
    [columns, density, overflow, sort.column, sort.direction],
  );
  const onSettings = useCallback((next: TableSettings) => {
    if (next.sort.column !== sort.column || next.sort.direction !== sort.direction) {
      setOption?.('tableSort', tableSortToken(next.sort));
    }
  }, [sort.column, sort.direction, setOption]);

  const checked = useMemo(
    () => (following && active && track ? gate.checkFollow({ panel: 'game-score', branchId: track.branch.id }) : null),
    [following, active, track?.branch.id, gate.checkFollow],
  );
  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(
    () => (track && checked?.query
      ? [{ name: 'main', source: track.source, branch: track.branch.id, offset: track.axisOffset, query: checked.query, channel: 'panel', product: 'game-score' }]
      : []),
    [track?.source, track?.branch.id, track?.axisOffset, checked?.query],
  );

  const pinnedReq = useMemo(
    () => (!following && track && pinHour !== null ? forwardRequest(track.source, 'game-score', { branchId: track.branch.id, hour: pinHour }) : null),
    [following, track, pinHour],
  );
  const pinned = useFollowedProduct<GameScoreReply>(runtime.client, pinnedReq, [], active && !following, validateGameScore, track?.source);
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
    const req = forwardRequest(track.source, 'game-score', { branchId: track.branch.id, hour: recorded });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await runtime.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map([['main', repliedCells(raw).read()]]) };
  }, [track, gate, runtime.client]);

  const projectionOptions = useMemo<GameScoreProjectionOptions>(() => ({
    all,
    chosenTags,
    membership,
    top: resolved.top ?? null,
    by,
    factions: options.factions === true,
    factionRuns,
    rankMarkers: options.rankMarkers === true,
    cellBars: options.cellBars === true,
    labels,
  }), [all, chosenTags, membership, resolved.top, by, options.factions, factionRuns, options.rankMarkers, options.cellBars, labels]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const contextMenuItems = useCallback(
    (_key: string, _name: string, atHour: number) => (track ? [jumpItem(runtime, track.id, atHour)] : []),
    [runtime, track],
  );

  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to see its game scores." />;
    if (coverage.outside) return <WidgetState kind="outside" read={coverage} />;
    if (needsGroups && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if ((needsGroups && !groups.data) || (bySlot && !slots)) return <WidgetState kind="loading" label="Loading game scores…" />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (following) {
      if (checked?.refused) return <WidgetState kind="unavailable" message={checked.refused} />;
      return null;
    }
    if (pinned.error && !pinned.data) return <WidgetState kind="error" failure={pinned.error} onRetry={pinned.retry} />;
    if (pinHour === null) return <WidgetState kind="outside" />;
    if (!pinned.data) return <WidgetState kind="loading" label="Loading game scores…" />;
    return null;
  })();

  return <FamilyTableHost
    key={track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : 'none'}
    declaration={gameScoreProjection}
    options={projectionOptions}
    label="Game score"
    subject="game scores"
    widget={widget}
    runtime={runtime}
    active={active}
    columns={columns}
    settings={tableSettings}
    onSettings={setOption ? onSettings : undefined}
    subscriptions={subscriptions}
    staticSources={staticSources}
    exactSources={track && perfLog && !resolved.reason ? exactSources : undefined}
    offset={track?.axisOffset ?? 0}
    pin={following ? null : pinHour}
    bounds={bounds}
    contextMenuItems={contextMenuItems}
    headerSlot={headerSlot}
    onDiagnostics={onDiagnostics}
    extraWarnings={groups.data?.warnings}
    blockedState={blockedState}
    loadingLabel="Loading game scores…"
  />;
}
