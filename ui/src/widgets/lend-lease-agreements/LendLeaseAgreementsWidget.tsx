import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createPortal } from 'react-dom';
import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { MissingName, ValueState } from '../../components/ValueState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { countryMatch, countryMatchChangeHours, membershipUnrecordedAt } from '../../dashboard/groups';
import { useChoiceHour } from '../../dashboard/atHead';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { LEASE_HISTORY, MARKET, partyTags, validateLeaseHistory, validateMarketReply } from '../../dashboard/market.ts';
import MarketDrill from '../../dashboard/MarketDrill';
import MarketEquipment from '../../dashboard/MarketEquipment';
import { nameReadMetadata, nameReadText } from '../../dashboard/nameReads.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { agreementRows, agreementValues, amountOf } from './agreementsData';
import type { AgreementRow, Direction } from './agreementsData';
import { AGREEMENT, DEFAULT_SORT, OPTIONS } from './definition';
import './lendLeaseAgreements.css';
import type { LendLeaseHistoryReply } from '../../generated/dashboard';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly AgreementRow[] = [];
const NO_AGREEMENTS: ReturnType<typeof agreementRows> = { rows: [], unattributedActive: 0, unattributedEnded: 0, unattributed: 0, membership: null };
const NO_WARNINGS: readonly string[] = [];
const rowKey = (row: AgreementRow) => row.key;
const rowOf = (key: string) => key.slice(0, key.lastIndexOf('\u0000'));

export default function LendLeaseAgreementsWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const [drillShown, setDrillShown] = useState<number | undefined>();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const direction: Direction = options.direction === 'received' ? 'received' : 'sent';
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => selectedCountryKeys(widget.countries, track), [widget.countries, track]);
  const tags = useMemo(() => widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? [...fallback].sort() : [], [widget.countries.kind, fallback]);
  const params = useMemo(() => track ? { countries: tags.map(countryFromKey), only: 'agreements' } : null, [track, tags]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), MARKET, params, { active, validate: validateMarketReply });
  const showEnded = options.ended !== 'hidden';
  const history = useProduct<LendLeaseHistoryReply>(runtime.client, track && showEnded ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: LEASE_HISTORY, params: { branchId: track.branch.id } }) : null, active, validateLeaseHistory, track?.source);
  const data = useMemo(() => head.data && head.at !== null ? { ...head.data, hour: head.at } : undefined, [head.data, head.at]);
  const hour = data?.hour ?? track?.branch.lastHour ?? 0;
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const unrecorded = useCallback((at: number) => membershipUnrecordedAt(widget.countries, groups.data, at), [widget.countries, groups.data]);
  const choiceAt = useChoiceHour(runtime.clock, active, () => head.atNow() ?? hour, () => head.shownAt(), countryMatchChangeHours(match.matches), head.live.subscribe);
  const listed = useMemo(() => data && match.ready && !match.reason
    ? agreementRows({ ...data, hour: choiceAt }, history.data, direction, match.matches, showEnded, unrecorded, head.live.read())
    : NO_AGREEMENTS, [data, history.data, match, direction, showEnded, unrecorded, choiceAt, head.live]);
  const rows = listed.rows;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const live = useMemo(() => agreementValues(rows, reporting(head.live, () => shown(shownAt.current()))), [rows, head.live, shown]);
  const warnings = useMemo(() => {
    const out = [...data?.warnings ?? [], ...history.data?.warnings ?? []];
    if (history.error) out.push(`Ended agreements: ${history.error.message}`);
    if (listed.unattributed) out.push(`${listed.unattributed} agreements whose ${direction === 'sent' ? 'givers' : 'receivers'} were not recorded at this date may also match`);
    return out.length ? out : NO_WARNINGS;
  }, [data, history.data, history.error, listed.unattributed, direction]);
  useDiagnostics(onDiagnostics, warnings);
  const several = widget.countries.kind !== 'selected' && !(widget.countries.kind === 'tags' && widget.countries.tags.length === 1);
  const [visible, setVisible] = useState<readonly AgreementRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((shownRows: readonly AgreementRow[]) => setVisible(shownRows), []);
  const wanted = useMemo(() => visible.flatMap(row => [row.giver, row.receiver].flatMap(tag => tag ? [{ hour: row.at, tag }] : [])), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const sortingNames = useMemo(() => sort.column === 'partner' ? rows.flatMap(row => [row.giver, row.partner].flatMap(tag => tag ? [{ hour: row.at, tag }] : [])) : [], [rows, sort.column]);
  const names = useCountryNamesAt(runtime.client, track, sortingNames, active);
  const name = useCallback((tag: string | null, at: number) => tag === null ? 'Country name unavailable' : nameReadText(names.get(nameKey(at, tag))), [names]);
  const countryLabel = useCallback((tag: string | null, at: number) => {
    if (tag === null) return nameReadMetadata(undefined);
    const read = names.get(nameKey(at, tag));
    return read ? { ...labelAt(at)(tag), ...nameReadMetadata(read) } : labelAt(at)(tag);
  }, [names, labelAt]);
  const title = useCallback((tag: string | null, at: number) => countryLabel(tag, at).name, [countryLabel]);
  const byKey = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const columns = useMemo<readonly TableColumn<AgreementRow>[]>(() => {
    const country = (tag: string | null, at: number, reason?: string | null) => tag === null
      ? (reason ? <ValueState kind="missing" narrow reason={reason} /> : <MissingName reason="Country name unavailable" />)
      : <CountryCell {...labelAt(at)(tag)} />;
    const header = several ? 'From → to' : direction === 'sent' ? 'Sent to' : 'Received from';
    const amount = (value: unknown, key: string) => amountOf(byKey.get(rowOf(key)) ?? { fuel: false }, typeof value === 'bigint' ? value : BigInt(Math.trunc(Number(value))));
    const missingAmount = (_key: string, reason: string) => reason === 'Not in this agreement' ? '—' : 'Not recorded';
    return [
      { key: 'partner', header, label: header, width: 'minmax(var(--lend-lease-country-min, var(--table-name-min)), 1.4fr)', renderMissing: true,
        value: row => { const tag = several ? row.giver : row.partner; return tag === null ? null : name(tag, row.at); },
        text: row => several ? `${title(row.giver, row.at)} → ${row.receiver === null && row.terms.receiverMissing ? row.terms.receiverMissing : title(row.receiver, row.at)}`
          : row.partner === null && row.partnerMissing ? row.partnerMissing : title(row.partner, row.at),
        render: row => several ? <span className="lend-lease-pair">{country(row.giver, row.at)}<span className="lend-lease-to">→ {country(row.receiver, row.at, row.terms.receiverMissing)}</span></span>
          : country(row.partner, row.at, row.partnerMissing) },
      { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(calc(var(--table-name-min) + 3em), 1.4fr)', renderMissing: true,
        value: row => row.items[0]?.name ?? null, text: row => row.items.map(item => item.name ?? item.missing ?? 'Equipment name unavailable').join(', '),
        render: row => <MarketEquipment name={row.items[0]?.name ?? null} icon={row.items[0]?.icon ?? null} missing={row.items[0]?.missing}
          detail={row.items.length > 1 ? `+${row.items.length - 1}` : null} /> },
      { key: 'schedule', header: 'Schedule', label: 'Schedule', width: 'minmax(120px, 1fr)', priority: 1, value: row => row.items[0]?.schedule || null,
        renderMissing: true,
        text: row => [row.items[0]?.schedule, row.items[0]?.scheduleMissing].filter(Boolean).join(' · ') || 'No active schedule',
        render: row => {
          const item = row.items[0];
          if (item?.schedule && item.scheduleMissing) return <ValueState kind="partial" narrow text={item.schedule} reason={item.scheduleMissing} />;
          if (item?.schedule) return item.schedule;
          return item?.scheduleMissing ? <ValueState kind="missing" narrow reason={item.scheduleMissing} /> : <span title="No active schedule">None</span>;
        } },
      { key: 'delivered', header: 'Delivered', label: 'Delivered', numeric: true, width: 'max-content', value: () => null, paint: amount, missingText: missingAmount },
      { key: 'lost', header: 'Lost', label: 'Lost', numeric: true, width: 'max-content', priority: 2, value: () => null, paint: amount, missingText: missingAmount },
      { key: 'last', header: 'Last delivery', label: 'Last delivery', numeric: true, width: 'max-content', priority: 3, renderMissing: true,
        value: row => row.last, text: row => row.last === null ? (row.lastMissing ?? 'No delivery yet') : dayLabel(row.last),
        render: row => row.last === null ? (row.lastMissing ? <ValueState kind="missing" narrow reason={row.lastMissing} /> : <span title="No delivery yet">No delivery</span>) : dayLabel(row.last) },
      { key: 'since', header: 'Since', label: 'Since', numeric: true, width: 'max-content', priority: 4, renderMissing: true,
        value: row => row.since, text: row => row.since === null ? (row.sinceMissing ?? 'Start date not recorded') : dayLabel(row.since),
        render: row => row.since === null ? <ValueState kind="missing" narrow reason={row.sinceMissing ?? 'Start date not recorded'} /> : dayLabel(row.since) },
    ];
  }, [several, direction, name, title, labelAt, byKey]);
  const tooltip = useCallback((row: AgreementRow) => agreementTooltip(row, countryLabel, viewport, live.read()), [countryLabel, viewport, live]);
  const menu = useCallback((row: AgreementRow) => track ? [row.since, row.last, row.ended ? row.at : null]
    .flatMap(at => at === null ? [] : [jumpItem(runtime, track.id, at)]) : [], [runtime, track]);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const pickOwner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [pick, setPick] = useState<{ owner: string | null; row: AgreementRow } | null>(null);
  const picked = pick && pick.owner === pickOwner ? pick.row : null;
  const open = (row: AgreementRow) => {
    setPick({ owner: pickOwner, row });
    if (row.ended) drillControl.showAt(AGREEMENT.id, row.at); else drillControl.show(AGREEMENT.id, row.at);
  };
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillOpen = drill.breakdown !== null && !!picked;
  const drillAt = useRegionHour(runtime, drillRegion, active && drillOpen);
  const drillDay = drillShown ?? drillAt;
  const drillTags = picked ? partyTags(picked.giver, picked.receiver) : null;
  const active_ = new Set(rows.filter(row => !row.ended).map(row => row.terms.key)).size;

  if (!track) return <WidgetState kind="empty" icon="link" message="Load a campaign to see lend-lease." />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const parent = head.outside ? <WidgetState kind="outside" read={head} />
    : head.error && !data ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
    : groupFailure && !groups.data ? groupFailure
    : match.reason ? <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>
    : !data || !match.ready ? <>{groupFailure}<div ref={head.root} className="lend-lease-agreements-host"><WidgetState kind="loading" label="Loading lend-lease…" /></div></> : null;
  if (parent && !drillOpen) return parent;
  const subject = fallback.length === 1 ? name(fallback[0], hour) : 'These countries';
  const body = !data ? null : data.agreements === null ? <WidgetState kind="unavailable" message="Lend-lease terms were not recorded." />
    : !rows.length ? listed.membership ? <WidgetState kind="unavailable" message={listed.membership} />
      : listed.unattributed ? <WidgetState kind="unavailable" message={`Some agreements' ${direction === 'sent' ? 'givers' : 'receivers'} were not recorded at this date.`} />
      : showEnded && !history.data && !history.error ? <WidgetState kind="loading" label="Loading ended agreements…" />
        : <WidgetState kind="empty" icon="link" message={`${subject} ${fallback.length === 1 ? 'has' : 'have'} no ${direction === 'sent' ? 'outgoing' : 'incoming'} lend-lease at this date.`} />
      : <Table rowRevision={row => [row, several, direction, ...[row.giver, row.receiver, row.partner].map(tag => tag === null ? null : labelAt(row.at)(tag))]}
        key={`${track.source.id}:${track.branch.id}`} label="Lend-lease agreements" rows={rows} columns={columns} rowKey={rowKey}
        sort={sort} setOption={setOption} cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
        rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} narrow={narrowTableOf(options)} muted={row => row.ended}
        onRowClick={open} menu={menu} tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} live={live} />;
  const main = parent ?? <div className="lend-lease-agreements" ref={head.root}>
    <div className="lend-lease-caption-line">
      <span>{dayLabel(hour)}</span>
    </div>
    <div className="lend-lease-agreements-table">{body}</div>
  </div>;
  const activePill = listed.unattributedActive
    ? listed.unattributedEnded
      ? `${active_.toLocaleString()}+ active · ${listed.unattributedActive.toLocaleString()} active, ${listed.unattributedEnded.toLocaleString()} ended unattributed`
      : `${active_.toLocaleString()}+ active · ${listed.unattributedActive.toLocaleString()} unattributed`
    : listed.unattributedEnded
      ? `${active_.toLocaleString()} active · ${listed.unattributedEnded.toLocaleString()} ended unattributed`
      : `${active_.toLocaleString()} active`;
  return <div ref={anchor} className="lend-lease-agreements-host">{parent ? null : groupFailure}
    {headerSlot && data && data.agreements !== null && createPortal(<span className="pill">{activePill}</span>, headerSlot)}
    <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={picked ? `${title(picked.giver, picked.at)} → ${title(picked.receiver, picked.at)}` : AGREEMENT.label} day={drillDay == null ? '' : dayLabel(drillDay)} canFollow onClose={() => dispatch({ type: 'close' })}
      main={main}>
      {picked && (drillTags ? <MarketDrill runtime={runtime} widget={widget} time={drillTime} active={active} onAnswered={setDrillShown}
        tags={drillTags} selected={`agreement:${picked.terms.id}`} />
        : <WidgetState kind="unavailable" message="Neither party was recorded at this hour." />)}
    </DrillHost>
  </div>;
}

export function agreementTooltip(row: AgreementRow, countryLabel: (tag: string | null, at: number) => ReturnType<CountryLabel>, viewport: number,
  values: { get(key: string): number | bigint | undefined; raw?(key: string): bigint | undefined; missing?(key: string): string }) {
  const lines: TooltipRow[] = [];
  const read = (column: string) => {
    const key = `${row.key}\u0000${column}`;
    const value = values.raw?.(key) ?? values.get(key);
    return value === undefined ? null : typeof value === 'bigint' ? value : BigInt(Math.trunc(value));
  };
  const missingOf = (column: string, fallback: string | null) => fallback ?? values.missing?.(`${row.key}\u0000${column}`) ?? 'Not in this agreement';
  row.items.forEach((item, index) => {
    const delivered = index === 0 ? read('delivered') : item.delivered, lost = index === 0 ? read('lost') : item.lost;
    lines.push({ label: item.name ?? item.missing ?? 'Equipment name unavailable', strong: true, lookupKey: item.variant ?? undefined });
    if (item.schedule) lines.push({ label: 'Schedule', value: item.schedule });
    if (item.scheduleMissing) lines.push({ label: item.schedule ? 'Schedule unavailable' : 'Schedule', value: item.scheduleMissing, tone: 'muted' });
    lines.push(delivered === null ? { label: 'Delivered', value: missingOf('delivered', item.deliveredMissing), tone: 'muted' }
      : { label: 'Delivered', value: amountOf(row, delivered) });
    lines.push(lost === null ? { label: 'Lost', value: missingOf('lost', item.lostMissing), tone: 'muted' }
      : { label: 'Lost', value: amountOf(row, lost) });
    if (delivered !== null && lost !== null && delivered + lost > 0n)
      lines.push({ label: 'Share lost', value: `${(Number(lost * 10000n / (delivered + lost)) / 100).toFixed(2)}%`, tone: 'muted' });
  });
  if (row.since !== null) lines.push({ label: 'Active since', value: dayLabel(row.since) });
  else if (row.sinceMissing) lines.push({ label: 'Active since', value: row.sinceMissing, tone: 'muted' });
  if (row.last !== null) lines.push({ label: 'Last delivery', value: dayLabel(row.last) });
  else if (row.lastMissing) lines.push({ label: 'Last delivery', value: row.lastMissing, tone: 'muted' });
  if (row.ended) lines.push({ label: `Last recorded ${dayLabel(row.at)}`, tone: 'muted' });
  const giver = countryLabel(row.giver, row.at), receiver = countryLabel(row.receiver, row.at);
  return fitTooltip([{ title: `${giver.name} → ${row.receiver === null && row.terms.receiverMissing ? row.terms.receiverMissing : receiver.name}`, ...countryNameDetails(giver, receiver), rows: lines }], viewport);
}
