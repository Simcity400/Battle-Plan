import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import { ChipSet } from '../../components/ChipSet';
import { CountryFlagUrl } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import type { LogListSection } from '../../charts/log/LogList';
import LogGaps from '../../charts/log/LogGaps';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { aheadEnd, useRevealClock } from '../../charts/reveal';
import { useAheadHour } from '../../dashboard/aheadHour';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabelValue } from '../../dashboard/countryLabelData';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { loadTransientProduct, useProduct } from '../../dashboard/products';
import { asFailure, failureText } from '../../dashboard/queryError';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useLogPages } from '../../dashboard/useLogPages';
import type { LogCut, LogSpec } from '../../dashboard/useLogPages';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useDistinctState } from '../../dashboard/useDistinctState';
import type { Country, EspionageEndedReply, EspionageOperationsReply, OperationState } from '../../generated/dashboard';
import type { Failure } from '../../dashboard/queryError';
import { OPTIONS } from './definition';
import { completionRow, endedRows, endedThrough, filteredPages, jumpHours, operationChoices, operationName, operationProgress, operationState,
  operationTooltip, operationWording, outcomeText, paintOperations, progressText, typeTokens, underWayRows, underWayStructure, validateEnded,
  validateOperation, validateOperations } from './espionageData';
import type { OperationDetail, OperationFilter, OperationItem } from './espionageData';
import { useEspionageTypes, useEspionageTypeTokens } from './useEspionageTypes';
import './espionageLog.css';
import { productRequest } from '../../dashboard/request.ts';

function SectionNote({ text, detail, failure, retry, loading }: { text?: string; detail?: string | null; failure?: Failure; retry?: () => void; loading?: boolean }) {
  if (failure) return <WidgetState kind="error" scale="section" what="operations" failure={failure} onRetry={retry} />;
  return <span className="espionage-note" data-state={loading ? 'loading' : undefined} data-tooltip={detail ?? undefined}>{text}</span>;
}

const sameCountryRows = (a: readonly LogRow<OperationItem>[], b: readonly LogRow<OperationItem>[]) => a.length === b.length
  && a.every((row, index) => { const other = b[index]; return row.key === other.key && row.item.countryHour === other.item.countryHour
    && row.item.owner?.tag === other.item.owner?.tag && row.item.target?.tag === other.item.target?.tag; });
const NO_ROWS: readonly LogRow<OperationItem>[] = [];
const COLUMNS = 'var(--space-6) minmax(0, 1fr)';
const METER_COLUMNS = 'var(--space-6) minmax(0, 2fr) minmax(0, 1fr)';
const UNKNOWN: CountryLabelValue = { name: 'Unknown country' };

export default function EspionageLogWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const settings = logSettings(options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const live = widget.time.mode === 'follow';
  const role = options.countryRole === 'target' || options.countryRole === 'either' ? options.countryRole : 'owner';
  const order = options.runningOrder === 'preparation' ? 'preparation' : 'start';
  const types = typeof options.types === 'string' ? options.types : '';
  const chosen = useMemo(() => { const keys = contentSet(types).map(contentKey); return keys.length ? new Set(keys) : null; }, [types]);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const selected = selectedCountryKeys(widget.countries, track);
  const selectedKey = JSON.stringify(selected);
  const fallback = useMemo(() => selected, [selectedKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, lastHour), [widget.countries, fallback, groups.data, lastHour]);
  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const pinned = widget.time.mode === 'pin' || widget.time.mode === 'range' ? window?.lastHour ?? null : null;
  const filter = useMemo<OperationFilter>(() => ({ types: chosen, role, country: match.matches, includeUnknown: widget.countries.kind === 'all', from }),
    [chosen, role, match, widget.countries.kind, from]);
  const typesRead = useEspionageTypes(widget, runtime, active);
  const tokensRead = useEspionageTypeTokens(widget, runtime, active);
  const byKey = useMemo(() => new Map(typesRead.data?.types.map(type => [type.key, type])), [typesRead.data]);
  const tokens = useMemo(() => typeTokens(tokensRead.data, chosen), [tokensRead.data, chosen]);

  const root = useRef<HTMLDivElement | null>(null);
  const latest = useRef<EspionageOperationsReply | null>(null);
  const paint = useCallback((reply: EspionageOperationsReply) => { latest.current = reply; paintOperations(root.current, reply); }, []);
  const structure = useCallback((reply: EspionageOperationsReply) => underWayStructure(reply, order), [order]);
  const params = useMemo(() => track ? {} : null, [track]);
  const gate = useGate(runtime, widget);
  const head = useHeadProduct(runtime, gate, 'espionage-operations', params, { active, validate: validateOperations, structure, paint });
  if (head.data && head.read !== 'followed') latest.current = head.data;
  useShownEachFrame(runtime.clock, () => head.shownAt(), head.live);
  const hour = head.at ?? head.hour ?? pinned ?? lastHour;
  const underWay = useMemo(() => match.ready ? underWayRows(head.data, hour, filter, order) : [], [head.data, hour, filter, order, match.ready]);
  const current = useCallback((item: OperationItem): OperationState | null => item.kind === 'preparing' || item.kind === 'running'
    ? latest.current?.operations.find(operation => operation.id === item.id)?.state ?? item.state : item.state, []);
  useEffect(() => { if (latest.current) paintOperations(root.current, latest.current); });

  const tagged = widget.countries.kind === 'tags' || widget.countries.kind === 'selected';
  const serverRole = tagged && role !== 'either' ? role : null;
  const facets = useMemo(() => [...(tokens ? [{ name: 'operation', values: tokens, countries: [] }] : []),
    ...(serverRole ? [{ name: serverRole, values: [], countries: fallback.map(countryFromKey) }] : [])], [tokens, serverRole, fallback]);
  const logSpan = track ? gate.span(from ?? track.branch.firstHour, track.branch.lastHour) : null;
  const spec = useMemo<LogSpec | null>(() => track && logSpan && tokens !== undefined && !(tokens && tokens.length === 0) ? { source: track.source, branchId: track.branch.id, log: 'operation-completions',
    product: 'espionage-log', ...(facets.length ? { facets } : {}), ...logSpan } : null,
  [track, tokens, facets, logSpan?.firstHour, logSpan?.lastHour]);
  const cut: LogCut = live ? { follow: true, offset: track?.axisOffset ?? 0 } : { follow: false, hour: widget.time.mode === 'campaign' ? null : pinned };
  const log = useLogPages<OperationItem>(runtime.client, runtime.clock, spec, cut, active, completionRow);
  const pageFilter = useMemo(() => needsGroups && !serverRole ? { ...filter, types: null, from: null } : null, [needsGroups, serverRole, filter]);
  const pages = useMemo(() => filteredPages(log.pages, pageFilter), [log.pages, pageFilter]);

  const endedTo = live || widget.time.mode === 'campaign' ? lastHour : pinned ?? lastHour;
  const endedAt = gate.within(endedTo);
  const endedFrom = from === null ? null : gate.within(from);
  const ended = useProduct<EspionageEndedReply>(runtime.client, track && endedAt !== null && (from === null || endedFrom !== null) ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'espionage-ended',
    params: { branchId: track.branch.id, from: endedFrom, to: endedAt } }) : null, active, validateEnded, track?.source);
  const offset = track?.axisOffset ?? 0;
  const endedHours = useMemo(() => ended.data ? ended.data.operations.map(operation => operation.byHour).reverse() : [], [ended.data]);
  const endedEnd = useCallback((clockHour: number) => {
    const { hoursPerSecond, playing } = runtime.clock.getSnapshot();
    return aheadEnd(endedHours, Math.floor(clockHour) + offset, hoursPerSecond, playing) - offset;
  }, [runtime.clock, endedHours, offset]);
  const endedBlock = useAheadHour(runtime.clock, active && live && !!ended.data, endedEnd);
  const endedShown = endedThrough(ended.data, endedBlock.until - 1 + offset);
  const trackHour = useCallback((clockHour: number) => Math.floor(clockHour) + offset, [offset]);
  const reveal = useRevealClock(runtime.clock, live, trackHour);
  const endedList = useMemo(() => match.ready ? endedRows(ended.data, live ? endedShown : null, filter) : NO_ROWS, [ended.data, live, endedShown, filter, match.ready]);

  const [visible, onVisible] = useDistinctState<readonly LogRow<OperationItem>[]>(NO_ROWS, sameCountryRows);
  const wanted = useMemo(() => {
    const seen = new Map<string, { hour: number; tag: string }>();
    for (const row of visible) for (const value of [row.item.owner, row.item.target]) {
      if (value?.tag) seen.set(JSON.stringify([row.item.countryHour, value.tag]), { hour: row.item.countryHour, tag: value.tag });
    }
    return [...seen.values()];
  }, [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const label = useCallback((value: Country | null, at: number): CountryLabelValue => !value ? UNKNOWN : !value.tag ? { name: `Country ${value.id}` } : labelAt(at)(value.tag), [labelAt]);

  const [tipRow, setTipRow] = useState<LogRow<OperationItem> | null>(null);
  const [detail, setDetail] = useState<{ key: string; value: OperationDetail } | null>(null);
  const detailHour = gate.recorded(tipRow ? tipRow.item.kind === 'ended' ? tipRow.item.lastSeen : tipRow.item.kind === 'completed' ? tipRow.hour : hour : null);
  const detailKey = tipRow?.item.id && detailHour !== null ? JSON.stringify([tipRow.item.id, detailHour]) : null;
  useEffect(() => {
    if (!active || !track || !tipRow?.item.id || detailHour === null || !detailKey) return;
    const controller = new AbortController();
    const id = tipRow.item.id;
    loadTransientProduct<unknown>(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'espionage-operation',
      params: { branchId: track.branch.id, id, hour: detailHour } }), controller.signal)
      .then(value => { if (!controller.signal.aborted) setDetail({ key: detailKey, value: { reply: validateOperation(value, id) } }); })
      .catch(error => { if (!controller.signal.aborted) setDetail({ key: detailKey, value: { failure: failureText(asFailure(error), 'Couldn’t load the operation.') } }); });
    return () => controller.abort();
  }, [active, track, runtime.client, tipRow, detailHour, detailKey]);
  const shownDetail = detail && detail.key === detailKey ? detail.value : null;

  const warnings = useMemo(() => [...new Set([...head.data?.warnings ?? [], ...ended.data?.warnings ?? [], ...typesRead.data?.warnings ?? [],
    ...typesRead.data?.missing ? [typesRead.data.missing] : [], ...tokensRead.data?.warnings ?? []])], [head.data, ended.data, typesRead.data, tokensRead.data]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <div ref={head.root} className="espionage-log"><WidgetState kind="empty" icon="target" message="Load a campaign to see its espionage operations." /></div>;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return <div ref={head.root} className="espionage-log">{groupFailure}</div>;
  if (match.reason) return <div ref={head.root} className="espionage-log">{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></div>;
  if (!match.ready) return <div ref={head.root} className="espionage-log"><WidgetState kind="loading" label="Loading espionage operations…" /></div>;

  const typeOf = (item: OperationItem) => item.operation === null ? undefined : byKey.get(item.operation);
  const typesPending = !typesRead.data && !typesRead.error;
  const renderRow = (row: LogRow<OperationItem>) => {
    const item = row.item;
    const type = typeOf(item);
    const owner = label(item.owner, item.countryHour), target = label(item.target, item.countryHour);
    const going = item.kind === 'preparing' || item.kind === 'running';
    const meter = going && live;
    const values = current(item);
    const wording = operationWording(item);
    const outcome = going ? live ? null : progressText(item, values) : outcomeText(item);
    const progress = operationProgress(item, values);
    return <>
      <span className="log-art" aria-hidden="true">{type?.icon ? <img src={type.icon} alt="" /> : <ArtFallback icon="target" reason={type?.missing ?? undefined} />}</span>
      <span className="log-text">
        {item.owner.tag && <CountryFlagUrl className="log-flag" url={owner.flag} detail={owner.flagDetail} reason={owner.flagReason} pending={owner.flagPending} />}<b>{owner.name}</b>
        <span className="espionage-muted">{wording.action}</span><span data-state={!type && typesPending ? 'loading' : undefined}>{operationName(type, typesPending)}</span>
        <span className="espionage-muted"> against </span>
        {item.target?.tag && <CountryFlagUrl className="log-flag" url={target.flag} detail={target.flagDetail} reason={target.flagReason} pending={target.flagPending} />}<b>{target.name}</b>
        <span className="espionage-muted">{wording.afterTarget}</span>
        {item.startHour !== null && <span className="espionage-muted"> · started {formatDateTime(item.startHour)}</span>}
        {outcome && <span className="espionage-muted"> · {outcome}</span>}
      </span>
      {meter && <span className="espionage-progress" title={item.kind === 'running' ? 'Execution progress: elapsed / duration' : 'Preparation'}>
        <span className="espionage-meter" data-esp-op={item.id} data-esp-field="meter"><i style={{ width: `${Math.max(0, Math.min(1, progress ?? 0)) * 100}%` }} /></span>
        <span data-esp-op={item.id} data-esp-field="text">{progressText(item, values)}</span>
      </span>}
    </>;
  };
  const describe = (row: LogRow<OperationItem>) => `${label(row.item.owner, row.item.countryHour).name}${operationWording(row.item).action}${operationName(typeOf(row.item), typesPending)
  } against ${label(row.item.target, row.item.countryHour).name}${operationWording(row.item).afterTarget}. ${operationState(row.item)}.`;
  const tooltip = (row: LogRow<OperationItem>) => [operationTooltip(row.item, row.hour, typeOf(row.item), value => label(value, row.item.countryHour),
    row.item.kind === 'preparing' || row.item.kind === 'running' ? current(row.item) : null, tipRow?.key === row.key ? shownDetail : null, typesPending)];
  const menu = (row: LogRow<OperationItem>) => jumpHours(row.item, row.hour).map(at => jumpItem(runtime, track.id, at));
  const onTooltip = (row: LogRow<OperationItem> | null) => setTipRow(row);
  const columns = (row: LogRow<OperationItem>) => (row.item.kind === 'preparing' || row.item.kind === 'running') && live ? METER_COLUMNS : COLUMNS;
  const chips = operationChoices(typesRead.data, tokensRead.data);
  const chosenChips = contentSet(types);
  const toggle = (value: string) => {
    if (value === '') { if (chosenChips.length) setOption?.('types', ''); return; }
    const next = toggleContentSet(types, value, Number.MAX_SAFE_INTEGER);
    if (next !== null) setOption?.('types', next);
  };
  const underWaySection: LogListSection<OperationItem> = head.error ? { key: 'under-way', label: 'Under way', months: false, rows: NO_ROWS, note: <SectionNote failure={head.error} retry={head.retry} /> }
    : !head.data ? { key: 'under-way', label: 'Under way', months: false, rows: NO_ROWS,
      note: head.read === 'off' ? <SectionNote text="This time is outside the recording." /> : <SectionNote text="Loading operations…" loading /> }
    : head.data.missing ? { key: 'under-way', label: 'Under way', months: false, rows: NO_ROWS, note: <SectionNote text="Operations were not recorded at this time." detail={head.data.missing} /> }
    : { key: 'under-way', label: 'Under way', months: false, rows: underWay, note: <SectionNote text="No operations under way match these countries and settings." /> };
  const completedSection: LogListSection<OperationItem> = log.error ? { key: 'completed', label: 'Completed', rows: NO_ROWS, note: <SectionNote failure={log.error} retry={log.retry} /> }
    : tokensRead.error && tokens === undefined ? { key: 'completed', label: 'Completed', rows: NO_ROWS, note: <SectionNote failure={tokensRead.error} retry={tokensRead.retry} /> }
    : tokens === undefined ? { key: 'completed', label: 'Completed', rows: NO_ROWS, note: <SectionNote text="Loading completed operations…" loading /> }
    : !spec ? { key: 'completed', label: 'Completed', rows: NO_ROWS, note: <SectionNote text="No operation of the chosen types was recorded." /> }
    : { key: 'completed', label: 'Completed', pages, note: <SectionNote text="No espionage operations match these countries and settings by this moment." /> };
  const endedSection: LogListSection<OperationItem> = ended.error ? { key: 'ended', label: 'Ended operations', rows: NO_ROWS, note: <SectionNote failure={ended.error} retry={ended.retry} /> }
    : !ended.data ? { key: 'ended', label: 'Ended operations', rows: NO_ROWS, note: <SectionNote text="Loading ended operations…" loading /> }
    : { key: 'ended', label: 'Ended operations', rows: endedList, note: <SectionNote text="No other endings recorded." /> };
  const sections = [underWaySection, completedSection, endedSection];

  return <div ref={element => { root.current = element; head.root(element); }} className="espionage-log">
    {groupFailure}
    {settings.chips && chips.length > 1 && <div className="log-chips">
      <ChipSet label="Operation types" max={Number.MAX_SAFE_INTEGER} selected={chosenChips.length ? chosenChips : ['']}
        chips={[{ value: '', label: 'All types' }, ...chips]} onToggle={toggle} />
    </div>}
    {spec && <LogGaps missing={log.missing} until={cut.follow ? hour : cut.hour} subject="Completed operations are not recorded whole" />}
    <LogList sections={sections} onWantedPages={log.onWantedPages} label="Espionage operations" density={settings.density} months={settings.months}
      animate={settings.animate} playhead={live ? hour : null} reveal={reveal} anchorRows renderRow={renderRow} describe={describe} tooltip={tooltip}
      menu={menu} onTooltip={onTooltip} columns={columns} onVisible={onVisible} />
  </div>;
}
