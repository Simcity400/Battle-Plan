import { preparedPrefix, pendingAt } from '../../dashboard/preparedPrefix';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { ChipSet } from '../../components/ChipSet';
import WidgetState from '../../components/WidgetState';
import { CountryFlagUrl } from '../../components/CountryFlag';
import LogList, { PreparedLogCoverage } from '../../charts/log/LogList';
import type { LogListProps } from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { jumpItem } from '../../charts/jump';
import { runAt } from '../../charts/series';
import { loadOperationDescription } from '../../dashboard/operationDescription';
import type { OperationDescriptionRef } from '../../generated/dashboard';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useParityRows } from '../../dashboard/parityRows';
import { parityWindowCoverage, parityWindowIndices, useParityPreparation } from '../../dashboard/parityPreparation';
import { OPTIONS } from './definition';
import { operationChoices, operationJumpHours, operationName, windowedOperationRows, operationWindowIndex, operationWindows, operationState, operationTooltip, operationWording, operationProgress, operationOutcome } from './espionageData';
import type { OperationAt } from './espionageData';
import { useEspionageLog, useEspionageWindows } from './useEspionageLog';
import './espionageLog.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<OperationAt>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr)';
const METER_COLUMNS = 'var(--space-6) minmax(0, 2fr) minmax(0, 1fr)';
const drawsMeter = (at: OperationAt, live: boolean) => live && (at.state === 'preparing' || at.state === 'running');

export default function EspionageLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const { track, data, error, retry } = useEspionageLog(widget, runtime, active);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const inBranch = track !== undefined && hour != null && hour >= track.branch.firstHour && hour <= track.branch.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const selected = selectedTags(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const types = typeof options.types === 'string' ? options.types : '';
  const role = options.countryRole === 'target' || options.countryRole === 'either' ? options.countryRole : 'owner';
  const live = widget.time.mode === 'follow';
  const runningOrder = options.runningOrder === 'preparation' ? 'preparation' : 'start';
  const includeUnknownCountries = widget.countries.kind === 'all';
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const parity = import.meta.env.MODE === 'parity' ? useParityPreparation() : null;
  const parityId = import.meta.env.MODE === 'parity' ? useId() : undefined;
  const playheadWindow = operationWindowIndex(data?.windows ?? [], hour ?? 0);
  const [browse, setBrowse] = useState<{ identity: string | null; hour: number | null }>({ identity: null, hour: null });
  const shownHour = parity?.requested ? hour : browse.identity === identity && browse.hour !== null ? Math.min(browse.hour, hour ?? browse.hour) : hour;
  const browseIndex = operationWindowIndex(data?.windows ?? [], shownHour ?? 0);
  const [visibleWindows, setVisibleWindows] = useState<{ identity: string | null; at: number; windows: number[] }>({ identity: null, at: -1, windows: [] });
  const visibleIndices = visibleWindows.identity === identity && visibleWindows.at === playheadWindow ? visibleWindows.windows : [];
  const completeWindows = import.meta.env.MODE === 'parity'
    ? parityWindowIndices(data?.windows, from ?? track?.branch.firstHour ?? 0, hour ?? 0) : [];
  const wantedWindows = parity?.requested ? completeWindows : operationWindows(data, hour ?? 0, live,
    [...visibleIndices, ...(browse.identity === identity && browse.hour !== null ? [browseIndex] : [])], from ?? -Infinity);
  const history = useEspionageWindows(track, runtime, wantedWindows, active && !!data);
  const rows = useMemo(() => inBranch && shownHour != null && match.ready
    ? windowedOperationRows(history.data, shownHour, { types: contentSet(types).map(contentKey), role, country: match.matches, from, live, runningOrder, includeUnknownCountries }) : NO_ROWS,
  [history.data, inBranch, shownHour, match, types, role, from, live, runningOrder, includeUnknownCountries]);
  const [visible, setVisible] = useState<readonly LogRow<OperationAt>[]>(NO_ROWS);
  const currentRows = useMemo(() => new Map(rows.map(row => [row.id, row])), [rows]);
  const drawn = useMemo(() => visible.flatMap(row => { const current = currentRows.get(row.id); return current ? [current] : []; }), [visible, currentRows]);
  const onVisible = useCallback((drawn: readonly LogRow<OperationAt>[]) => {
    setVisible(previous => {
      const next = drawn.map(row => currentRows.get(row.id) ?? row);
      return previous.length === next.length && previous.every((row, index) => row === next[index]) ? previous : next;
    });
    if (parity?.requested) return;
    const windows = [...new Set(drawn.flatMap(row => row.item.windowIndex === undefined ? [] : [row.item.windowIndex]))].sort((a, b) => a - b);
    setVisibleWindows(previous => previous.identity === identity && previous.at === playheadWindow && JSON.stringify(previous.windows) === JSON.stringify(windows)
      ? previous : { identity, at: playheadWindow, windows });
  }, [currentRows, identity, playheadWindow, parity?.requested]);
  const [tooltipRow, setTooltipRow] = useState<LogRow<OperationAt> | null>(null);
  const [description, setDescription] = useState<{ reference: OperationDescriptionRef; text: string; detail?: string } | null>(null);
  const descriptionRef = tooltipRow && shownHour != null && (tooltipRow.item.operation.end?.byHour ?? Infinity) <= shownHour
    && (tooltipRow.item.state === 'completed' || tooltipRow.item.state === 'disappeared')
    ? tooltipRow.item.operation.end?.outcome?.descriptionRef ?? null : null;
  const onTooltip = useCallback((row: LogRow<OperationAt> | null) => {
    setTooltipRow(row);
    if (!row) setDescription(null);
  }, []);
  const sourceId = track?.source.id, revision = track?.source.revision;
  const descriptionSource = useMemo(() => sourceId !== undefined && revision !== undefined ? { sourceId, revision } : null,
    [sourceId, revision]);
  const branchId = track?.branch.id;
  const descriptionLanguage = tooltipRow?.item.operation.end?.outcome?.recordedLanguage;
  useEffect(() => {
    if (!active || !descriptionRef || !descriptionSource || !branchId) return;
    const controller = new AbortController();
    loadOperationDescription(runtime.client, descriptionSource, branchId, descriptionRef, controller.signal)
      .then(result => {
        if (result.recordedLanguage !== descriptionLanguage)
          throw new Error('The stored description language differs from its outcome.');
        if (!controller.signal.aborted) setDescription({ reference: descriptionRef, text: result.text });
      })
      .catch(error => { if (!controller.signal.aborted) setDescription({ reference: descriptionRef,
        text: 'Couldn’t load description.', detail: error instanceof Error ? error.message : String(error) }); });
    return () => controller.abort();
  }, [active, runtime.client, descriptionRef, descriptionSource, branchId, descriptionLanguage]);
  // Names and flags belong to the endpoints' supplying read, including an ending's upper bound.
  const wanted = useMemo(() => {
    const seen = new Map<string, { hour: number; tag: string }>();
    for (const row of drawn) for (const tag of [row.item.ownerTag, row.item.targetTag]) {
      if (tag !== null) seen.set(JSON.stringify([row.item.countryHour, tag]), { hour: row.item.countryHour, tag });
    }
    return [...seen.values()];
  }, [drawn]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const labels = useCallback((row: LogRow<OperationAt>, tag: string | null): ReturnType<CountryLabel> => tag === null ? { name: 'Unknown country' } : labelAt(row.item.countryHour)(tag), [labelAt]);
  const chips = useMemo(() => operationChoices(data), [data]);
  const byType = useMemo(() => new Map(data?.types.map(type => [type.key, type])), [data]);
  let parityHistoryComplete = false;
  let paritySemantic: LogListProps<OperationAt>['paritySemantic'];
  if (import.meta.env.MODE === 'parity') {
    const coverage = parityWindowCoverage(completeWindows, history.data, history.errors);
    parityHistoryComplete = !!data && coverage.complete;
    const complete = parity!.requested && parityHistoryComplete && match.ready;
    paritySemantic = { complete, preparation: { id: parityId, prepare: parity!.prepare, ready: () => complete,
      error: () => error ?? coverage.error },
      content: row => { const type = row.item.typeKey === null ? null : byType.get(row.item.typeKey); return type ? { key: type.key, name: type.name } : null; },
      reads: { hour, from, windows: coverage.outcomes } };
  }
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'log', label: 'Espionage operation values',
    complete: Boolean(data && match.ready && (parity?.requested ? parityHistoryComplete : history.data.has(browseIndex))), total: rows.length,
    rows: rows.map(row => ({ id: row.id, hour: row.hour, by: row.by, state: row.item.state, startHour: row.item.startHour,
      readHour: row.item.readHour, countryHour: row.item.countryHour, ownerTag: row.item.ownerTag, targetTag: row.item.targetTag,
      typeKey: row.item.typeKey, run: row.item.run, end: row.item.operation.end && row.item.operation.end.byHour <= (shownHour ?? -Infinity) ? row.item.operation.end : null })),
  }), paritySemantic?.preparation ? { ...paritySemantic.preparation, id: `${parityId}:values` } : undefined);
  const warnings = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...[...history.data.values()].flatMap(reply => reply.warnings),
    ...(description?.detail ? [description.detail] : [])])], [data, history.data, description?.detail]);
  useEffect(() => {
    onDiagnostics(warnings);
    return () => onDiagnostics(NO_WARNINGS);
  }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="target" message="Load a campaign to see its espionage operations." />;
  const windowError = history.errors.values().next().value;
  if (windowError) return <WidgetState kind="error" detail={windowError} onRetry={history.retry} />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (needsGroups && groups.error) return <WidgetState kind="error" detail={`Country identities are unavailable: ${groups.error}`} onRetry={groups.retry} />;
  if (!data || (needsGroups && !groups.data)) return <WidgetState kind="loading" label="Loading espionage operations…" />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (preparedPrefix(data)?.pending.length && !data.available.length && !data.completionsAvailable.length) return <PreparedLogCoverage coverage={preparedPrefix(data)?.coverage} />;
  if (data.unavailable && !data.completionsAvailable.length) return <WidgetState kind="unavailable" message="Operations unavailable." />;
  if (!inBranch || hour == null) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!data.available.length && !data.completionsAvailable.length) return <WidgetState kind="unavailable" message="Espionage operations are not in this recording." />;
  if (browseIndex >= 0 && !history.data.has(browseIndex)) return <WidgetState kind="loading" label="Loading espionage window..." />;

  const preceding = wantedWindows.filter(index => index <= browseIndex);
  const oldest = preceding.length ? Math.min(...preceding) : 0;
  const earlier = oldest > 0 && data.windows[oldest - 1].lastHour >= (from ?? -Infinity);
  const navigation = <div className="espionage-navigation">
    {earlier && <button type="button" onClick={() => { setBrowse({ identity, hour: data.windows[oldest - 1].lastHour }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Earlier operations</button>}
    {shownHour !== hour && <button type="button" onClick={() => { setBrowse({ identity, hour: null }); setVisibleWindows({ identity, at: playheadWindow, windows: [] }); }}>Latest operations</button>}
  </div>;
  const unread = !runAt(data.available, shownHour ?? hour) && !runAt(data.completionsAvailable, shownHour ?? hour);
  const chosen = contentSet(types);
  const toggle = (value: string) => { const next = toggleContentSet(types, value, Number.MAX_SAFE_INTEGER); if (next !== null) setOption?.('types', next); };
  const renderRow = (row: LogRow<OperationAt>) => {
    const at = row.item;
    const type = at.typeKey === null ? undefined : byType.get(at.typeKey);
    const owner = labels(row, at.ownerTag);
    const target = labels(row, at.targetTag);
    const meter = drawsMeter(at, live);
    const wording = operationWording(at);
    const outcome = operationOutcome(at);
    return <>
      <span className="log-art" aria-hidden="true">{type?.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="target" />}</span>
      <span className="log-text">
        {at.ownerTag !== null && <CountryFlagUrl className="log-flag" url={owner.flag} reason={owner.flagReason} pending={owner.flagPending} />}<b>{owner.name}</b>
        <span className="espionage-muted">{wording.action}</span>{operationName(type)}<span className="espionage-muted"> against </span>
        {at.targetTag !== null && <CountryFlagUrl className="log-flag" url={target.flag} reason={target.flagReason} pending={target.flagPending} />}<b>{target.name}</b><span className="espionage-muted">{wording.afterTarget}</span>
        {at.startHour !== null && <span className="espionage-muted"> · started {formatDateTime(at.startHour)}</span>}
        {!meter && outcome && <span className="espionage-muted"> · {outcome}</span>}
      </span>
      {meter && <span className="espionage-progress">
        <ProgressMeter value={operationProgress(at)} state="live" label={at.state === 'running' ? 'Execution progress: elapsed / duration' : 'Preparation'} />
      </span>}
    </>;
  };
  return <div ref={root} className="espionage-log">
    <PreparedLogCoverage coverage={preparedPrefix(data)?.coverage} exact={(history.data.get(browseIndex) as { exact?: number } | undefined)?.exact} />
    {navigation}
    {shownHour !== hour && <p className="espionage-muted">Showing operations through {formatDateTime(shownHour!)}</p>}
    {log.chips && chips.length > 1 && <div className="log-chips">
      <ChipSet label="Operation types" max={Number.MAX_SAFE_INTEGER} selected={chosen.length ? chosen : ['']}
        chips={[{ value: '', label: 'All types' }, ...chips]}
        onToggle={value => { if (value === '') { if (chosen.length) setOption?.('types', ''); } else toggle(value); }} />
    </div>}
    <LogList anchorRows key={`${track.source.id}:${track.branch.id}`} rows={rows} label="Espionage operations" density={log.density} months={log.months}
      {...(import.meta.env.MODE === 'parity' ? { parityHistoryComplete, paritySemantic } : {})}
      animate={log.animate && live && shownHour === hour} columns={row => drawsMeter(row.item, live) ? METER_COLUMNS : COLUMNS} renderRow={renderRow}
      describe={row => `${labels(row, row.item.ownerTag).name}${operationWording(row.item).action}${operationName(row.item.typeKey === null ? undefined : byType.get(row.item.typeKey))} against ${labels(row, row.item.targetTag).name}${operationWording(row.item).afterTarget}. ${operationState(row.item)}.`}
      onTooltip={onTooltip}
      tooltip={row => operationTooltip(data, row, tag => labels(row, tag),
        description?.reference === row.item.operation.end?.outcome?.descriptionRef ? description?.text : undefined)}
      menu={row => operationJumpHours(row.item, shownHour ?? hour, row.hour).map(at => jumpItem(runtime, track.id, at))}
      empty={pendingAt(preparedPrefix(history.data.get(browseIndex)), shownHour ?? hour) ? "This range is still being prepared." : unread ? "Operations unavailable at this time." : "No espionage operations match these countries and settings by this moment."} onVisible={onVisible} />
  </div>;
}
