import { useCallback, useEffect, useMemo, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { ChipSet } from '../../components/ChipSet';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import LogList from '../../charts/log/LogList';
import type { LogListProps } from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken, storableToken, toggleContentSet, UNSTORABLE } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useProductWindows } from '../../dashboard/productWindows';
import { useContentRevision } from '../../dashboard/contentRevision';
import { battleWindowIndex, battleWindows } from '../land-battles/landBattlesData';
import { sinkingWindowRows } from './shipsSunkWindows';
import type { SinkingItem } from './shipsSunkWindows';
import type { ShipsSunkWindowReply } from './useShipsSunk';
import { OPTIONS } from './definition';
import { shipBattleLabel, shipClassChoices, shipClassName, shipCountryLabels, shipDescription, shipName, shipPlace, shipTooltip } from './shipsSunkData';
import { useShipsSunk, useShipsSunkBattleTitles, validateSinkingWindow } from './useShipsSunk';
import { parityWindowCoverage, parityWindowIndices, useParityPreparation } from '../../dashboard/parityPreparation';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<SinkingItem>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr)';

export default function ShipsSunkWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const manifest = useShipsSunk(widget, runtime, active);
  const { track, error, retry } = manifest;
  const data = manifest.data?.classes;
  const contentRevision = useContentRevision(runtime.client);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const classes = typeof options.classes === 'string' ? options.classes : '';
  const role = options.countryRole === 'victim' || options.countryRole === 'sinker' ? options.countryRole : 'both';
  const campaignKey = track?.source.metadata.campaignKey ?? '';
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, contentRevision]) : null;
  const coveredHour = manifest.data?.coverage && !manifest.data.coverage.complete ? Math.min(hour ?? manifest.data.coverage.lastHour, manifest.data.coverage.lastHour) : hour;
  const playheadWindow = battleWindowIndex(manifest.data?.windows ?? [], coveredHour ?? 0);
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const shownHour = !parity?.requested && browse.identity === identity && browse.hour !== null ? Math.min(browse.hour, coveredHour ?? browse.hour) : coveredHour;
  const browseIndex = battleWindowIndex(manifest.data?.windows ?? [], shownHour ?? 0);
  const [visibleWindows, setVisibleWindows] = useState<{ identity: string | null; at: number; windows: number[] }>({ identity: null, at: -1, windows: [] });
  const visibleIndices = visibleWindows.identity === identity && visibleWindows.at === playheadWindow ? visibleWindows.windows : [];
  const parityWindows = import.meta.env.MODE === 'parity' ? parityWindowIndices(manifest.data?.windows, from ?? Number.MIN_SAFE_INTEGER, hour ?? 0) : [];
  const wantedWindows = parity?.requested ? parityWindows : battleWindows(manifest.data, coveredHour ?? 0, widget.time.mode === 'follow',
    [...visibleIndices, ...(browse.identity === identity && browse.hour !== null ? [browseIndex] : [])], from ?? Number.MIN_SAFE_INTEGER);
  const history = useProductWindows<ShipsSunkWindowReply>(runtime.client, identity, index => ({ sourceId: track!.source.id, revision: track!.source.revision,
    product: 'ships-sunk', params: { branchId: track!.branch.id, hour: manifest.data!.windows[index].lastHour } }), wantedWindows,
  active && data?.available === true, validateSinkingWindow, track?.source, true);
  const country = useCallback((tag: string | null, namingHour: number) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, namingHour),
    [match, widget.countries.kind]);
  const rows = useMemo(() => shownHour != null && match.ready ? sinkingWindowRows(history.data, shownHour,
    contentSet(classes).map(contentKey), role, country, from, campaignKey) : NO_ROWS,
  [history.data, shownHour, match.ready, classes, role, country, from, campaignKey]);
  const [visible, setVisible] = useState<readonly LogRow<SinkingItem>[]>(NO_ROWS);
  const currentRows = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const drawn = useMemo(() => visible.flatMap(row => { const current = currentRows.get(row.key); return current ? [current] : []; }), [visible, currentRows]);
  const titleRows = useMemo(() => drawn.map(row => row.item.index).sort((a, b) => a - b), [drawn]);
  const titles = useShipsSunkBattleTitles(runtime, track, titleRows, active);
  const titleByRow = useMemo(() => new Map(titles.data?.rows.map(row => [String(row.index), row])), [titles.data]);
  const wanted = useMemo(() => drawn.flatMap(({ item: row }) => [
    { hour: row.victimAsOf ?? row.sinkHour, tag: row.victim },
    { hour: row.attackerAsOf ?? row.sinkHour, tag: row.attacker },
  ].filter((entry): entry is { hour: number; tag: string } => entry.tag !== null)), [drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: SinkingItem) => shipCountryLabels(row, labelAt), [labelAt]);
  let paritySemantic: LogListProps<SinkingItem>['paritySemantic'];
  if (import.meta.env.MODE === 'parity') {
    const coverage = parityWindowCoverage(parityWindows, history.data, history.errors);
    const fullRows = useMemo(() => hour != null && match.ready ? sinkingWindowRows(history.data, hour, contentSet(classes).map(contentKey), role,
      country, from, campaignKey) : NO_ROWS, [history.data, hour, match.ready, classes, role, country, from, campaignKey]);
    const complete = parity!.requested && !!data && coverage.complete && match.ready;
    paritySemantic = { rows: fullRows, complete, preparation: { prepare: parity!.prepare, ready: () => complete, error: () => error ?? coverage.error },
      item: row => { const { page, index: _index, windowIndex: _window, ...item } = row.item;
        return { ...item, classIndex: data!.classes.findIndex(type => type.token === page.classes[item.classIndex].token),
          recordedNameIndex: item.recordedNameIndex === null ? null : data!.recordedNames.indexOf(page.recordedNames[item.recordedNameIndex]),
          placeIndex: item.placeIndex === null ? null : data!.places.indexOf(page.places[item.placeIndex]) }; },
      content: row => { const page = row.item.page; return { class: { token: page.classes[row.item.classIndex].token,
        key: page.classes[row.item.classIndex].key, name: page.classes[row.item.classIndex].name },
        recordedName: row.item.recordedNameIndex === null ? null : page.recordedNames[row.item.recordedNameIndex],
        place: row.item.placeIndex === null ? null : page.places[row.item.placeIndex], battleTitleIndex: row.item.index }; },
      reads: data ? { branchId: data.branchId, firstHour: data.firstHour, lastHour: data.lastHour,
        firstRead: data.firstRead, unread: data.unread, titleReferenceCount: fullRows.length } : null };
  }
  const onVisible = useCallback((next: readonly LogRow<SinkingItem>[]) => {
    setVisible(previous => previous.length === next.length && previous.every((row, index) => row === next[index]) ? previous : next);
    if (parity?.requested) return;
    const windows = [...new Set(next.map(row => row.item.windowIndex))].sort((a, b) => a - b);
    setVisibleWindows(previous => previous.identity === identity && previous.at === playheadWindow && JSON.stringify(previous.windows) === JSON.stringify(windows)
      ? previous : { identity, at: playheadWindow, windows });
  }, [identity, playheadWindow, parity?.requested]);
  const chips = useMemo(() => shipClassChoices(data, campaignKey).map(choice => {
    const value = setToken(choice.value);
    return { ...choice, value, unavailable: storableToken(value) ? undefined : UNSTORABLE };
  }), [data, campaignKey]);
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...[...history.data.values()].flatMap(page => page.reply.warnings), ...(groups.data?.warnings ?? []), ...(titles.data?.warnings ?? [])];
    if (titles.error) warnings.push(`Ships sunk battle titles unavailable for ${titleRows.length} drawn rows: ${titles.error}`);
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, history.data, groups.data, titles.data, titles.error, titleRows.length, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see its ships sunk log." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading ships sunk…" /></>;
  if (!data.available) return <>{groupFailure}{manifest.data?.pending?.length ? <PreparedLogCoverage coverage={manifest.data.coverage} /> : <WidgetState kind="unavailable" message="Not in this recording" />}</>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour)
    return <>{groupFailure}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;

  const pageError = history.errors.get(browseIndex);
  if (pageError) return <>{groupFailure}<WidgetState kind="error" detail={pageError} onRetry={history.retry} /></>;
  if (browseIndex >= 0 && !history.data.has(browseIndex)) return <>{groupFailure}<WidgetState kind="loading" label="Loading ships sunk?" /></>;
  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? Math.min(...preceding) : 0;
  const earlier = oldest > 0 && manifest.data!.windows[oldest - 1].lastHour >= (from ?? Number.MIN_SAFE_INTEGER);
  const chosen = contentSet(classes);
  const renderRow = (logRow: LogRow<SinkingItem>) => {
    const row = logRow.item;
    const data = row.page;
    const type = data.classes[row.classIndex];
    const { victim, attacker } = labels(row);
    const battle = shipBattleLabel(row, titleByRow.get(logRow.key));
    const attackerFlag = row.attacker === null ? null : <CountryFlagUrl className="log-flag"
      url={attacker.flag} reason={attacker.flagReason} pending={attacker.flagPending} />;
    return <>
      <span className="log-art" aria-hidden="true">{type.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="anchor" />}</span>
      <span className="log-text">
        <b>{shipName(row)}</b>, {shipClassName(data, row)}{' · '}
        {row.victim !== null && <CountryFlagUrl className="log-flag" url={victim.flag} reason={victim.flagReason} pending={victim.flagPending} />}<b>{victim.name}</b>{', '}
        {row.cause === 'cause' ? <>lost to {row.attributionName ?? 'Not recorded'}{row.attacker !== null && <>; attributed to {attackerFlag}<b>{attacker.name}</b></>}</>
          : row.assistedKill || row.cause === 'assisted' ? <>sunk with assistance from {attackerFlag}<b>{attacker.name}</b></>
            : <>sunk by {attackerFlag}<b>{attacker.name}</b></>}
        {' in '}{shipPlace(data, row)}{battle && <>{', '}{battle}</>}
      </span>
    </>;
  };
  return <div className="log-widget">{groupFailure}
    <PreparedLogCoverage coverage={manifest.data?.coverage} />
    {manifest.data?.pending?.length ? <p className="chart-line-key">Class choices cover prepared history.</p> : null}
    {log.chips && chips.length > 1 && <div className="log-chips"><ChipSet label="Ship classes" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
      chips={[{ value: '', label: 'All classes' }, ...chips]} onToggle={value => {
        if (value === '') setOption?.('classes', '');
        else { const next = toggleContentSet(classes, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('classes', next); }
      }} /></div>}
    {(earlier || shownHour !== coveredHour) && <div className="log-navigation">
      {earlier && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: manifest.data!.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier sinkings</button>}
      {shownHour !== coveredHour && <button type="button" className="button small" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest sinkings</button>}
    </div>}
    <LogList rows={rows} label="Ships sunk" density={log.density} months={log.months} animate={log.animate && widget.time.mode === 'follow' && shownHour === hour}
      {...(import.meta.env.MODE === 'parity' ? { paritySemantic } : {})}
      columns={COLUMNS} renderRow={renderRow} anchorRows onVisible={onVisible}
      describe={row => { const named = labels(row.item); return shipDescription(row.item.page, row.item, named.victim.name, named.attacker.name, titleByRow.get(row.key)); }}
      tooltip={row => { const named = labels(row.item); return [shipTooltip(row.item.page, row.item, named.victim.name, named.attacker.name, titleByRow.get(row.key), titles.error)]; }}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty={match.known(from ?? track.branch.firstHour, shownHour ?? hour) ? "No ships sunk recorded for these settings." : MEMBERSHIP_PREPARING} />
  </div>;
}
