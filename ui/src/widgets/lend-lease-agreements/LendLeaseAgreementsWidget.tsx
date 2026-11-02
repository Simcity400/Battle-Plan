import { agreementRevision } from './agreementsData.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { nameReadText, nameReadMetadata } from '../../dashboard/nameReads.ts';
import { createPortal } from 'react-dom';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupReasonAt, preparedAt, valueAt } from '../../dashboard/economyHistory';
import { EquipmentCell, useVariantArt } from '../../dashboard/EquipmentCell';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import MarketDrill from '../../dashboard/MarketDrill';
import { knownAt, MARKET_HISTORY, partiesOf, partyTagsAt, rosterCount, validateMarketHistory } from '../../dashboard/marketHistory';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { MarketHistoryReply } from '../../generated/dashboard';
import { agreementRows, amountOf, firstAmount, openAgreement } from './agreementsData';
import type { AgreementRow, Direction } from './agreementsData';
import { AGREEMENT, DEFAULT_SORT, DIRECTIONS, OPTIONS } from './definition';
import '../equipment-stockpile/equipmentStockpile.css';
import './lendLeaseAgreements.css';

const NO_ROWS: readonly AgreementRow[] = [];
const NO_AGREEMENTS: ReturnType<typeof agreementRows> = { rows: [], unattributed: 0 };
const NO_WARNINGS: readonly string[] = [];
const rowKey = (row: AgreementRow) => row.key;
const validate = validateMarketHistory('agreements');

/** What each country sends or receives through lend-lease: schedule, delivered and
 * lost per agreement; ended agreements muted. A row opens Trade flows' agreement drill. */
export default function LendLeaseAgreementsWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  // The open drill may still show an earlier hour while its newest loads.
  const [drillShown, setDrillShown] = useState<number | undefined>();
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const direction: Direction = options.direction === 'received' ? 'received' : 'sent';
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const pickOwner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [pick, setPick] = useState<{ client: typeof runtime.client; owner: string | null; row: AgreementRow } | null>(null);
  const picked = pick && pick.client === runtime.client && pick.owner === pickOwner ? pick.row : null;
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => selectedTags(widget.countries, track), [widget.countries, track]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const tags = widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? fallback : [];
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: MARKET_HISTORY,
    params: { branchId: track.branch.id, view: 'agreements', tags: [...tags].sort() } } : null;
  const { data, error, retry } = useProduct<MarketHistoryReply>(runtime.client, request, active, validate, track?.source);
  const showEnded = options.ended !== 'hidden';
  const listed = useMemo(() => data && match.ready && !match.reason ? agreementRows(data, direction, match.matches, hour, showEnded) : NO_AGREEMENTS,
    [data, match, direction, hour, showEnded]);
  const rows = listed.rows;
  // An agreement whose side in this direction has no tag may be a chosen country's: counted, never listed.
  const warnings = useMemo(() => listed.unattributed
    ? [...data?.warnings ?? [], `${listed.unattributed} agreements whose ${direction === 'sent' ? 'givers' : 'receivers'} were not recorded at this date may also match`]
    : data?.warnings ?? NO_WARNINGS, [listed.unattributed, data, direction]);
  useDiagnostics(onDiagnostics, warnings);
  const several = widget.countries.kind !== 'selected' && !(widget.countries.kind === 'tags' && widget.countries.tags.length === 1);
  const [visible, setVisible] = useState<readonly AgreementRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((shown: readonly AgreementRow[]) => setVisible(shown), []);
  const wanted = useMemo(() => visible.flatMap(row => [row.from, row.partner].flatMap(tag => tag ? [{ hour: row.at, tag }] : [])), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'partner' || sort.column === 'from'
    ? rows.flatMap(row => [row.from, row.partner].flatMap(tag => tag ? [{ hour: row.at, tag }] : [])) : [], [rows, sort.column]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  // Any country recorded at the hour reads the variants' pictures; all countries or a group name none.
  const artTag = fallback[0] ?? visible.find(row => !row.ended && row.giver !== null)?.giver ?? null;
  const art = useVariantArt(runtime, track, artTag, hour, useMemo(() => visible.flatMap(row => row.items[0]?.variant ? [row.items[0].variant] : []), [visible]), active);
  const name = useCallback((tag: string | null, at: number) => tag === null ? 'Country name unavailable' : nameReadText(names.get(nameKey(at, tag))), [names]);
  // Titles read the shown rows' labels; sorting keeps the tag order until every name has arrived.
  const countryLabel = useCallback((tag: string | null, at: number) => {
    if (tag === null) return nameReadMetadata(undefined);
    const read = names.get(nameKey(at, tag));
    return read ? { ...labelAt(at)(tag), ...nameReadMetadata(read) } : labelAt(at)(tag);
  }, [names, labelAt]);
  const title = useCallback((tag: string | null, at: number) => countryLabel(tag, at).name, [countryLabel]);
  const columns = useMemo<readonly TableColumn<AgreementRow>[]>(() => {
    const country = (tag: string | null, at: number) => tag === null ? 'Country name unavailable' : <CountryCell {...labelAt(at)(tag)} />;
    const header = several ? 'From → to' : direction === 'sent' ? 'Sent to' : 'Received from';
    return [
      // The countries lead: a table's first column never hides. Several countries' agreements name both sides.
      { key: 'partner', header, label: header, width: 'minmax(var(--lend-lease-country-min, var(--table-name-min)), 1.4fr)', renderMissing: true,
        value: row => { const tag = several ? row.giver : row.partner; return tag === null ? null : name(tag, row.at); },
        text: row => several ? `${title(row.giver, row.at)} → ${title(row.receiver, row.at)}` : title(row.partner, row.at),
        render: row => several ? <span className="lend-lease-pair">{country(row.giver, row.at)}<span className="lend-lease-to">→ {country(row.receiver, row.at)}</span></span>
          : country(row.partner, row.at) },
      // A name beside its icon.
      { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(calc(var(--table-name-min) + 3em), 1.4fr)',
        value: row => row.items[0]?.name ?? null, renderMissing: true,
        text: row => row.items.map(item => item.name ?? 'Equipment name unavailable').join(', '),
        render: row => <EquipmentCell name={row.items[0]?.name ?? null} icon={row.items[0]?.variant ? art.get(row.items[0].variant) : null}
          detail={row.items.length > 1 ? `+${row.items.length - 1}` : null} /> },
      { key: 'schedule', header: 'Schedule', label: 'Schedule', width: 'minmax(120px, 1fr)', priority: 1, value: row => row.items[0]?.schedule || null,
        renderMissing: true, render: row => row.items[0]?.schedule || '—' },
      { key: 'delivered', header: 'Delivered', label: 'Delivered', numeric: true, width: 'max-content', renderMissing: true,
        value: row => firstAmount(row, 'delivered'), text: row => amountOf(row, firstAmount(row, 'delivered')),
        render: row => amountOf(row, firstAmount(row, 'delivered')) },
      { key: 'lost', header: 'Lost', label: 'Lost', numeric: true, width: 'max-content', priority: 2, renderMissing: true,
        value: row => firstAmount(row, 'lost'), text: row => amountOf(row, firstAmount(row, 'lost')), render: row => amountOf(row, firstAmount(row, 'lost')) },
      { key: 'last', header: 'Last delivery', label: 'Last delivery', numeric: true, width: 'max-content', priority: 3,
        value: row => row.last, text: row => row.last === null ? '—' : dayLabel(row.last), render: row => row.last === null ? '—' : dayLabel(row.last) },
      { key: 'since', header: 'Since', label: 'Since', numeric: true, width: 'max-content', priority: 4,
        value: row => row.since, text: row => row.since === null ? '—' : dayLabel(row.since), render: row => row.since === null ? '—' : dayLabel(row.since) },
    ];
  }, [several, direction, name, title, labelAt, art]);
  const tooltip = useCallback((row: AgreementRow) => agreementTooltip(row, countryLabel, viewport), [countryLabel, viewport]);
  const menu = useCallback((row: AgreementRow) => track ? [row.since, row.last, row.ended ? row.at : null]
    .flatMap(at => at === null ? [] : [jumpItem(runtime, track.id, at)]) : [], [runtime, track]);
  const playing = runtime.clock.getSnapshot().playing;
  const open = (row: AgreementRow) => {
    setPick({ client: runtime.client, owner: pickOwner, row });
    dispatch(openAgreement(row, AGREEMENT.id, playing, widget.time.mode === 'follow' ? hour : null));
  };
  const at = drillHour(drill, hour) ?? hour;
  const drillAt = drill.follow ? hour : at;
  const drillTags = picked && data ? partyTagsAt(partiesOf(data.holders, [picked.agreement.giver, valueAt(picked.agreement.receiver, picked.at)]), data.holders, drillAt) : null;
  // An agreement's fuel terms are a row of their own, not another agreement.
  const active_ = new Set(rows.filter(row => !row.ended).map(row => `${row.agreement.giver}:${row.agreement.id}`)).size;
  const activeCount = data ? rosterCount(data, hour, active_, listed.unattributed) : null;

  if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see lend-lease." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading lend-lease…" /></>;
  if (!preparedAt(data, hour)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  // A group whose membership at this hour is unrecorded or empty says so, never "none";
  // membership still being prepared at the hours the agreements are matched is neither.
  if (!rows.length && !match.known(track.branch.firstHour, hour)) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  const groupReason = groupReasonAt(widget.countries, groups.data, hour, track.branch.lastHour);
  if (groupReason) return <>{groupFailure}<WidgetState kind={groupReason.kind} message={groupReason.message} /></>;
  const subject = fallback.length === 1 ? name(fallback[0], hour) : 'These countries';
  const body = !knownAt(data, hour) ? <WidgetState kind="unavailable" message="Lend-lease terms were not recorded." />
    : !rows.length ? listed.unattributed ? <WidgetState kind="unavailable" message={`Some agreements' ${direction === 'sent' ? 'givers' : 'receivers'} were not recorded at this date.`} />
      : <WidgetState kind="empty" icon="link" message={`${subject} ${fallback.length === 1 ? 'has' : 'have'} no ${direction === 'sent' ? 'outgoing' : 'incoming'} lend-lease at this date.`} />
      : <Table rowRevision={row => [...agreementRevision(row), several, direction,
        ...[row.giver, row.receiver, row.partner].map(tag => tag === null ? null : labelAt(row.at)(tag)),
        row.items[0]?.variant ? art.get(row.items[0].variant) : null]} key={`${track.source.id}:${track.branch.id}`} label="Lend-lease agreements" rows={rows} columns={columns} rowKey={rowKey}
        sort={sort} setOption={setOption} cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
        rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} narrow={narrowTableOf(options)} muted={row => row.ended}
        onRowClick={open} menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />;
  const main = <div className="lend-lease-agreements">
    <div className="stockpile-caption-line">
      <span>{dayLabel(hour)}</span>
      <Segmented value={direction} options={DIRECTIONS} label="Direction" onChange={value => setOption?.('direction', value)} />
    </div>
    <div className="lend-lease-agreements-table">{body}</div>
  </div>;
  return <div ref={anchor} className="lend-lease-agreements-host">{groupFailure}
    {headerSlot && activeCount !== null && createPortal(<span className="pill">{activeCount} active</span>, headerSlot)}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked ? `${title(picked.giver, picked.at)} → ${title(picked.receiver, picked.at)}` : AGREEMENT.label} day={dayLabel(drillShown ?? at)}
      following={drill.follow} canFollow={widget.time.mode === 'follow'} onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })}
      main={main}>
      {picked && (drillTags ? <MarketDrill runtime={runtime} track={track} hour={drillAt} active={active} onAnswered={setDrillShown}
        tags={drillTags} selected={`agreement:${picked.agreement.id}`} />
        : <WidgetState kind="unavailable" message="Neither party was recorded at this hour." />)}
    </DrillHost>
  </div>;
}

export function agreementTooltip(row: AgreementRow, countryLabel: (tag: string | null, at: number) => ReturnType<CountryLabel>, viewport: number) {
  const lines: TooltipRow[] = [];
  for (const item of row.items) {
    lines.push({ label: item.name ?? 'Equipment name unavailable', strong: true, lookupKey: item.variant ?? undefined });
    if (item.schedule) lines.push({ label: 'Schedule', value: item.schedule });
    lines.push({ label: 'Delivered', value: amountOf(row, item.delivered) }, { label: 'Lost', value: amountOf(row, item.lost) });
    if (item.delivered !== null && item.lost !== null && item.delivered + item.lost > 0n)
      lines.push({ label: 'Share lost', value: `${(Number(item.lost * 10000n / (item.delivered + item.lost)) / 100).toFixed(2)}%`, tone: 'muted' });
  }
  if (row.since !== null) lines.push({ label: 'Active since', value: dayLabel(row.since) });
  if (row.last !== null) lines.push({ label: 'Last delivery', value: dayLabel(row.last) });
  if (row.ended) lines.push({ label: `Last recorded ${dayLabel(row.at)}`, tone: 'muted' });
  const giver = countryLabel(row.giver, row.at), receiver = countryLabel(row.receiver, row.at);
  return fitTooltip([{ title: `${giver.name} → ${receiver.name}`, ...countryNameDetails(giver, receiver), rows: lines }], viewport);
}
