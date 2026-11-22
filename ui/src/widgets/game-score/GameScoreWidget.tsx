import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import MeasureRace from '../../charts/race/MeasureRace';
import { formatRaw } from '../../charts/race/measureRaceData.ts';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { countryTagAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { isMember } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { usePushedLooksHour } from '../../dashboard/pushedLooks.ts';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { SCORE_LABELS, SCORE_PARTS, factionGroup, formatScore, scoreCountries, scoreHint, scoreParts, scorePart, scoreRows, scoreTooltip, validateGameScore } from './gameScoreData.ts';
import type { ScoreOf } from './gameScoreData';
import { liveKey } from '../../charts/live';
import { useHeadHour } from '../army-size/headHour';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { useShownHour, useWhyCommit } from '../../dashboard/widgetPerf.ts';
import type { GameScoreRow } from '../../generated/dashboard';
import type { GameScoreReply } from './gameScoreData';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly GameScoreRow[] = [];
const PARAMS = {};
const rowKey = (row: GameScoreRow) => row.tag;
const scoreStructure = (reply: GameScoreReply) => reply.rows.map(row => [row.tag, ...SCORE_PARTS.map(part => row[part].missing ?? '')].join('|')).join('\n');
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

function ScoreTable({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const by = scorePart(options.topMeasure);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const all = widget.countries.kind === 'all';
  const needsGroups = options.factions === true || (widget.countries.kind === 'group' && widget.countries.group.type !== 'top');
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const resolved = useMemo(() => scoreCountries(widget.countries, [...selectedCountryKeys(widget.countries, track)], groups.data, lastHour),
    [widget.countries, track, groups.data, lastHour]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'game-score', track ? PARAMS : null, { active, validate: validateGameScore, structure: scoreStructure });
  const reply = head.data;
  const live = head.live;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const score = useCallback<ScoreOf>((tag, part) => live.read().get(liveKey(tag, part)), [live]);
  const hour = head.at ?? lastHour;
  const bySlot = resolved.tags.some(isSlotKey);
  const slots = useSlotTags(runtime.client, track, active && bySlot);
  const chosenTags = useMemo(() => bySlot ? resolved.tags.flatMap(key => countryTagAt(slots, countryFromKey(key), hour) ?? []) : resolved.tags,
    [bySlot, resolved.tags, slots, hour]);
  const membership = resolved.membership;
  const member = useMemo(() => membership ? (tag: string, hour: number) => isMember(membership.get(tag), hour) : null, [membership]);
  const top = resolved.top ?? null;
  const woken = useMemo(() => new Set<() => void>(), []);
  useEffect(() => live.subscribe(() => { for (const wake of [...woken]) wake(); }), [live, woken]);
  const ranked = useHeadHour(runtime.clock, active && top != null && !!reply, () => reply && top != null
    ? scoreRows(reply, { all, tags: chosenTags, top, hour }, member, by, score).map(row => row.tag).sort().join(',') : '', woken);
  const tableLive = useMemo(() => reporting(live, () => shown(shownAt.current())), [live, shown, ranked]);
  const members = reply && top != null ? scoreRows(reply, { all, tags: chosenTags, top, hour }, member, by, score) : null;
  const membersKey = members?.map(row => row.tag).sort().join(',') ?? '';
  const rows = useMemo(() => members ?? (reply ? scoreRows(reply, { all, tags: chosenTags, top, hour }, member, by, score) : NO_ROWS),
    [reply, all, chosenTags, top, member, by, score, membersKey, hour]);
  const [visible, setVisible] = useState<readonly GameScoreRow[]>(NO_ROWS);
  const pushed = head.read === 'followed';
  const rowTagsKey = (pushed ? rows : visible).map(row => row.tag).sort().join(',');
  const rowTags = useMemo(() => rowTagsKey ? rowTagsKey.split(',') : [], [rowTagsKey]);
  const nameHour = usePushedLooksHour(runtime, widget, track, rowTags, active && pushed, hour).hour;
  const labels = useCountryLabelsAt(runtime.client, track, useMemo(() => rowTags.map(tag => ({ tag, hour: nameHour })), [rowTags, nameHour]), active);
  const label = labels.labelAt(nameHour);
  useWhyCommit('ScoreTable', { reply, at: head.at, read: head.read, live, ranked, groups: groups.data, visible, nameHour, labels: labels.labelAt });
  const hints = reply?.hints;
  const hintsKey = JSON.stringify(hints ?? null);
  const columns = useMemo<readonly TableColumn<GameScoreRow>[]>(() => [
    { key: 'country', header: SCORE_LABELS.country, label: SCORE_LABELS.country, width: 'minmax(var(--table-country-min), 2fr)', hint: scoreHint(hints && { hints }, 'country'),
      value: row => label(row.tag).name, render: row => { const look = label(row.tag); return <CountryCell name={look.name} flag={look.flag}
        flagReason={look.flagReason} flagDetail={look.flagDetail} flagPending={look.flagPending} />; } },
    ...SCORE_PARTS.map((part, index) => ({ key: part, header: SCORE_LABELS[part], label: SCORE_LABELS[part], numeric: true, width: 'max-content',
      bar: 'visible' as const, priority: index + 1, inTooltip: true, hint: scoreHint(hints && { hints }, part),
      value: () => null, paint: (value: unknown) => formatScore(value as number) })),
  ], [label, hintsKey]);
  const tooltip = useCallback((row: GameScoreRow) => fitTooltip([scoreTooltip(reply!, row, label(row.tag).name, dayLabel(hour), factionGroup(groups.data, row.tag, hour).label, score)],
    viewport, { keep: SCORE_PARTS.length }), [reply, label, hour, groups.data, viewport, score]);
  useEffect(() => {
    const warnings = [...(reply?.warnings ?? []), ...(groups.data?.warnings ?? [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [reply, groups.data, onDiagnostics]);
  const state = (() => {
    if (!track) return <WidgetState kind="empty" icon="grid" message="Load a campaign to see its game scores." />;
    if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
    if (needsGroups && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (head.read === 'off') return <WidgetState kind="outside" />;
    if (!reply || (needsGroups && !groups.data) || (bySlot && !slots)) return <WidgetState kind="loading" label="Loading game scores…" />;
    if (reply.missing) return <WidgetState kind="unavailable" message={reply.missing} />;
    if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
    if (!rows.length) return <WidgetState kind="unavailable" message={`No country’s scores recorded on ${dayLabel(hour)}.`} />;
    return undefined;
  })();
  return <div ref={head.root} style={{ display: 'contents' }}>{state ?? <Table key={JSON.stringify([track!.source.id, track!.source.revision, track!.branch.id])}
    label="Game score" rows={rows} columns={columns} rowKey={rowKey} tieKey={rowKey} sort={sort} setOption={setOption} sortOptionValue={tableSortToken}
    rowRevision={row => [label(row.tag), ...SCORE_PARTS.map(part => row[part].missing)]}
    groupBy={options.factions === true ? row => factionGroup(groups.data, row.tag, hour) : undefined}
    cellBars={options.cellBars === true} rank rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} tooltip={tooltip} onVisibleRows={pushed ? undefined : setVisible}
    menu={() => [jumpItem(runtime, track!.id, hour)]} live={tableLive} />}</div>;
}
