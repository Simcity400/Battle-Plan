import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { aheadEnd, useRevealClock } from '../../charts/reveal';
import { useAheadHour } from '../../dashboard/aheadHour';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useProductWindows } from '../../dashboard/productWindows';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { battleWindowIndex, battleWindows, heldWindowsEnd } from '../battles/battleRows';
import { sinkingWindowRows } from './shipsSunkWindows';
import type { SinkingItem } from './shipsSunkWindows';
import type { ShipsSunkWindowReply } from './useShipsSunk';
import { OPTIONS } from './definition';
import { manifestClassChoices, shipClassName, shipCountryLabels, shipDescription, shipName, shipPlace, shipTooltip } from './shipsSunkData';
import { useShipsSunk, validateSinkingWindow } from './useShipsSunk';
import { useDistinctState } from '../../dashboard/useDistinctState';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<SinkingItem>[] = [];
type VisibleWindows = { identity: string | null; at: number; windows: number[] };
const NO_VISIBLE_WINDOWS: VisibleWindows = { identity: null, at: -1, windows: [] };
const sameWindows = (held: VisibleWindows, next: VisibleWindows) => held.identity === next.identity && held.at === next.at
  && held.windows.length === next.windows.length && held.windows.every((window, index) => window === next.windows[index]);
const sameRows = <R,>(held: readonly R[], next: readonly R[]) => held.length === next.length && held.every((row, index) => row === next[index]);
const COLUMNS = 'var(--space-6) minmax(0, 1fr)';
const NO_PAGES: ReadonlyMap<number, ShipsSunkWindowReply> = new Map();

export default function ShipsSunkWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const manifest = useShipsSunk(widget, runtime, active);
  const { track, error, retry } = manifest;
  const data = manifest.data;
  const contentRevision = useContentRevision(runtime.client);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const following = active && widget.time.mode === 'follow';
  const windowAt = useCallback((clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour) : null, [widget, track, runtime.tracks]);
  const trackHour = useCallback((clockHour: number) => Math.floor(windowAt(clockHour)?.lastHour ?? clockHour), [windowAt]);
  const windows = manifest.data?.windows;
  const blockInputs = useRef<{ trackHour: typeof trackHour; windows: readonly { firstHour: number; lastHour: number }[]; pages: ReadonlyMap<number, ShipsSunkWindowReply>; edges: readonly number[] }>(
    { trackHour, windows: [], pages: NO_PAGES, edges: [] });
  const end = useCallback((clockHour: number) => {
    const { trackHour: at, windows: spans, pages: held, edges: hours } = blockInputs.current;
    const base = at(clockHour);
    const { hoursPerSecond, playing } = runtime.clock.getSnapshot();
    return Math.floor(clockHour) + Math.min(aheadEnd(hours, base, hoursPerSecond, playing), heldWindowsEnd(spans, held, base)) - base;
  }, [runtime.clock]);
  const block = useAheadHour(runtime.clock, following, end);
  const window = windowAt(block.hour);
  const hour = window ? Math.floor(window.lastHour) : undefined;
  const drawnThrough = hour === undefined ? undefined : following ? hour + block.until - 1 - Math.floor(block.hour) : hour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const classes = typeof options.classes === 'string' ? options.classes : '';
  const role = options.countryRole === 'victim' || options.countryRole === 'sinker' ? options.countryRole : 'both';
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision]) : null;
  const coveredHour = hour;
  const playheadWindow = battleWindowIndex(manifest.data?.windows ?? [], coveredHour ?? 0);
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const shownHour = browse.identity === identity && browse.hour !== null ? Math.min(browse.hour, coveredHour ?? browse.hour) : coveredHour;
  const browseIndex = battleWindowIndex(manifest.data?.windows ?? [], shownHour ?? 0);
  const [visibleWindows, setVisibleWindows] = useDistinctState<VisibleWindows>(NO_VISIBLE_WINDOWS, sameWindows);
  const visibleIndices = visibleWindows.identity === identity ? visibleWindows.windows : [];
  const wantedWindows = battleWindows(manifest.data, coveredHour ?? 0, widget.time.mode === 'follow',
    [...visibleIndices, ...(browse.identity === identity && browse.hour !== null ? [browseIndex] : [])], from ?? Number.MIN_SAFE_INTEGER);
  const history = useProductWindows<ShipsSunkWindowReply>(runtime.client, identity, index => (productRequest({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'ships-sunk', params: { branchId: track!.branch.id, window: index }, asksFrom: manifest.data?.windows[index]?.firstHour })), wantedWindows,
  active && data?.available === true, validateSinkingWindow, track?.source, true);
  const edges = useMemo(() => [...new Set([...(windows ?? []).map(window => window.firstHour),
    ...[...history.data.values()].flatMap(page => page.reply.rows.map(row => Math.max(row.sinkHour, row.knownAt)))])].sort((a, b) => a - b),
  [windows, history.data]);
  blockInputs.current = { trackHour, windows: windows ?? [], pages: history.data, edges };
  const browsing = shownHour !== coveredHour;
  const reveal = useRevealClock(runtime.clock, following && !browsing, trackHour);
  useShownEachFrame(runtime.clock, () => shownHour == null ? null : reveal ? trackHour(runtime.clock.getHour()) : shownHour);
  const country = useCallback((tag: string | null, namingHour: number) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, namingHour),
    [match, widget.countries.kind]);
  const rowsThrough = reveal ? drawnThrough : shownHour;
  const rows = useMemo(() => rowsThrough != null && match.ready ? sinkingWindowRows(history.data, rowsThrough,
    contentSet(classes).map(contentKey), role, country, from, campaignKey) : NO_ROWS,
  [history.data, rowsThrough, match.ready, classes, role, country, from, campaignKey]);
  const [visible, setVisible] = useDistinctState<readonly LogRow<SinkingItem>[]>(NO_ROWS, sameRows);
  const currentRows = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const drawn = useMemo(() => reveal ? rows : visible.flatMap(row => { const current = currentRows.get(row.key); return current ? [current] : []; }),
    [reveal, rows, visible, currentRows]);
  const wanted = useMemo(() => drawn.flatMap(({ item: row }) => [
    { hour: row.victimAsOf ?? row.sinkHour, tag: row.victim },
    { hour: row.attackerAsOf ?? row.sinkHour, tag: row.attacker },
  ].filter((entry): entry is { hour: number; tag: string } => entry.tag !== null)), [drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: SinkingItem) => shipCountryLabels(row, labelAt), [labelAt]);
  const onVisible = useCallback((next: readonly LogRow<SinkingItem>[]) => {
    if (!reveal) setVisible(next);
    const windows = [...new Set(next.map(row => row.item.windowIndex))].sort((a, b) => a - b);
    if (reveal && windows.every(index => wantedWindows.includes(index))) return;
    setVisibleWindows({ identity, at: playheadWindow, windows });
  }, [reveal, identity, playheadWindow, wantedWindows, setVisible, setVisibleWindows]);
  const chips = useMemo(() => manifestClassChoices(data, campaignKey).map(choice => {
    const value = setToken(choice.value);
    return { ...choice, value, unavailable: storableToken(value) ? undefined : UNSTORABLE };
  }), [data, campaignKey]);
  useEffect(() => {
    const warnings = [...new Set([...(data?.warnings ?? []), ...[...history.data.values()].flatMap(page => page.reply.warnings), ...(groups.data?.warnings ?? [])])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, history.data, groups.data, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see its ships sunk log." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading ships sunk…" /></>;
  if (!data.available) return <>{groupFailure}<WidgetState kind="unavailable" message="Not in this recording" /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour)
    return <>{groupFailure}<WidgetState kind="outside" /></>;

  const pageError = history.errors.get(browseIndex);
  if (pageError) return <>{groupFailure}<WidgetState kind="error" failure={pageError} onRetry={history.retry} /></>;
  if (browseIndex >= 0 && !history.data.has(browseIndex)) return <>{groupFailure}<WidgetState kind="loading" label="Loading ships sunk…" /></>;
  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? Math.min(...preceding) : 0;
  const earlier = oldest > 0 && manifest.data!.windows[oldest - 1].lastHour >= (from ?? Number.MIN_SAFE_INTEGER);
  const chosen = contentSet(classes);
  const renderRow = (logRow: LogRow<SinkingItem>) => {
    const row = logRow.item;
    const data = row.page;
    const type = data.classes[row.classIndex];
    const { victim, attacker } = labels(row);
    const attackerFlag = row.attacker === null ? null : <CountryFlagUrl className="log-flag"
      url={attacker.flag} detail={attacker.flagDetail} reason={attacker.flagReason} pending={attacker.flagPending} />;
    return <>
      <span className="log-art" aria-hidden="true">{type.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="anchor" />}</span>
      <span className="log-text">
        <b>{shipName(row)}</b>, {shipClassName(data, row)}{' · '}
        {row.victim !== null && <CountryFlagUrl className="log-flag" url={victim.flag} detail={victim.flagDetail} reason={victim.flagReason} pending={victim.flagPending} />}<b>{victim.name}</b>{', '}
        {row.cause === 'cause' ? <>lost to {row.attributionName ?? 'Not recorded'}{row.attacker !== null && <>; attributed to {attackerFlag}<b>{attacker.name}</b></>}</>
          : row.assistedKill || row.cause === 'assisted' ? <>sunk with assistance from {attackerFlag}<b>{attacker.name}</b></>
            : <>sunk by {attackerFlag}<b>{attacker.name}</b></>}
        {' in '}{shipPlace(data, row)}
      </span>
    </>;
  };
  return <div className="log-widget">{groupFailure}
    {log.chips && chips.length > 1 && <div className="log-chips"><ChipSet label="Ship classes" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All classes' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('classes', '');
        else { const next = toggleContentSet(classes, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('classes', next); }
      }} /></div>}
    {(earlier || browsing) && <div className="log-navigation">
      {earlier && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: manifest.data!.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier sinkings</button>}
      {browsing && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest sinkings</button>}
    </div>}
    <LogList rows={rows} label="Ships sunk" density={log.density} months={log.months} animate={log.animate && widget.time.mode === 'follow' && shownHour === hour}
      playhead={reveal ? hour : undefined} reveal={reveal} columns={COLUMNS} renderRow={renderRow} anchorRows onVisible={onVisible}
      describe={row => { const named = labels(row.item); return shipDescription(row.item.page, row.item, named.victim.name, named.attacker.name); }}
      tooltip={row => { const named = labels(row.item); return [{ ...shipTooltip(row.item.page, row.item, named.victim.name, named.attacker.name), ...countryNameDetails(named.victim, named.attacker) }]; }}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty="No ships sunk recorded for these settings." />
  </div>;
}
