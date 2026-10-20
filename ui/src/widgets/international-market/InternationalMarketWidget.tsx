import { createPortal } from 'react-dom';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import Table, { CellBar, CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupReasonAt, preparedAt } from '../../dashboard/economyHistory';
import { EquipmentCell, EQUIPMENT_UNNAMED, useVariantArt } from '../../dashboard/EquipmentCell';
import { countryMatch } from '../../dashboard/groups';
import MarketDrill from '../../dashboard/MarketDrill';
import { knownAt, MARKET_HISTORY, validateMarketHistory } from '../../dashboard/marketHistory';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { MarketHistoryReply } from '../../generated/dashboard';
import { DETAILS, OPTIONS, SORTS, VIEWS } from './definition';
import { cic, contractRows, firstWithMore, levelText, offerRows, units } from './marketData';
import type { ContractRow, MarketTab, OfferRow } from './marketData';
import '../equipment-stockpile/equipmentStockpile.css';
import './internationalMarket.css';

const NO_WARNINGS: readonly string[] = [];
const VALIDATORS = { contracts: validateMarketHistory('contracts'), requests: validateMarketHistory('requests'), inventory: validateMarketHistory('inventory') };

/** Who buys what from whom on the international market at the moment shown:
 * contracts, open requests or listed inventory. A row opens Trade flows' market drill. */
export default function InternationalMarketWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view: MarketTab = options.view === 'requests' ? 'requests' : options.view === 'inventory' ? 'inventory' : 'contracts';
  const sort = tableSortOf(options[`sort-${view}`], SORTS[view]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [picked, setPicked] = useState<{ title: string; tags: string[]; selected: string | null } | null>(null);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => selectedTags(widget.countries, track), [widget.countries, track]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const tags = widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? fallback : [];
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: MARKET_HISTORY,
    params: { branchId: track.branch.id, view, tags: [...tags].sort() } } : null;
  const { data, error, retry } = useProduct<MarketHistoryReply>(runtime.client, request, active, VALIDATORS[view], track?.source);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const ready = !!data && data.view === view && match.ready && !match.reason;
  const contracts = useMemo(() => ready && view !== 'inventory' ? contractRows(data!, hour, match.matches) : [], [ready, data, view, hour, match]);
  const offers = useMemo(() => ready && view === 'inventory' ? offerRows(data!, hour, match.matches) : [], [ready, data, view, hour, match]);
  const [visibleTags, setVisibleTags] = useState<readonly string[]>([]);
  const [visibleVariants, setVisibleVariants] = useState<readonly string[]>([]);
  const onVisibleContracts = useCallback((rows: readonly ContractRow[]) => {
    setVisibleTags(rows.flatMap(row => [row.seller, row.buyer].filter((tag): tag is string => tag !== null)));
    setVisibleVariants(rows.flatMap(row => row.equipment[0] ? [row.equipment[0].variant] : []));
  }, []);
  const onVisibleOffers = useCallback((rows: readonly OfferRow[]) => {
    setVisibleTags(rows.flatMap(row => row.seller ? [row.seller] : []));
    setVisibleVariants(rows.map(row => row.variant));
  }, []);
  const wanted = useMemo(() => visibleTags.map(tag => ({ hour, tag })), [visibleTags, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const allTags = useMemo(() => [...new Set([...contracts.flatMap(row => [row.seller, row.buyer]), ...offers.map(row => row.seller)]
    .filter((tag): tag is string => tag !== null))].map(tag => ({ hour, tag })), [contracts, offers, hour]);
  const names = useCountryNamesAt(runtime.client, track, sort.column === 'seller' || sort.column === 'buyer' ? allTags : [], active);
  const art = useVariantArt(runtime, track, fallback[0] ?? visibleTags[0] ?? null, hour, visibleVariants, active);
  const name = useCallback((tag: string | null) => tag === null ? 'Country name unavailable' : names.get(nameKey(hour, tag)) ?? tag, [names, hour]);
  const country = useCallback((key: 'seller' | 'buyer', header: string, priority?: number) => ({ key, header, label: header, priority,
    width: 'minmax(4em, 1fr)', renderMissing: true,
    value: (row: ContractRow | OfferRow) => (row as ContractRow)[key] ?? null, text: (row: ContractRow | OfferRow) => name((row as ContractRow)[key] ?? null),
    render: (row: ContractRow | OfferRow) => { const tag = (row as ContractRow)[key] ?? null; return tag === null ? 'Country name unavailable' : <CountryCell {...labelAt(hour)(tag)} />; } }), [name, labelAt, hour]);
  const contractColumns = useMemo<readonly TableColumn<ContractRow>[]>(() => view === 'contracts' ? [
    country('seller', 'Seller'), country('buyer', 'Buyer'),
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1.4fr)', renderMissing: true,
      value: row => row.equipment[0]?.name ?? null, text: row => row.equipment.map(item => item.name ?? EQUIPMENT_UNNAMED).join(', '),
      render: row => <EquipmentCell name={row.equipment[0]?.name ?? null} icon={row.equipment[0] ? art.get(row.equipment[0].variant) : null} detail={firstWithMore(row.equipment)} /> },
    { key: 'price', header: 'Price', label: 'Agreed price', numeric: true, width: 'max-content', priority: 1, renderMissing: true,
      value: row => row.prices[0]?.amount ?? null, text: row => row.prices.map(item => cic(item.amount)).join(', ') || '—', render: row => cic(row.prices[0]?.amount ?? null) },
    { key: 'convoys', header: 'Convoys', label: 'Convoys assigned', numeric: true, width: 'max-content', priority: 3, renderMissing: true,
      value: row => row.assigned, text: row => row.assigned === null ? '—' : `${row.assigned} / ${row.convoys ?? '—'}`,
      render: row => row.assigned === null ? '—' : `${row.assigned} / ${row.convoys ?? '—'}` },
    { key: 'collected', header: 'Collected', label: 'Payment collected', numeric: true, width: 'max-content', renderMissing: true,
      value: row => row.collected, text: row => cic(row.collected), render: row => cic(row.collected) },
    { key: 'deliveries', header: 'Deliveries', label: 'Completed deliveries', numeric: true, width: 'max-content', priority: 4, renderMissing: true,
      value: row => row.deliveries, render: row => row.deliveries === null ? '—' : row.deliveries.toString() },
  ] : [
    country('buyer', 'Buyer'), country('seller', 'Seller', 2),
    { key: 'equipment', header: 'Wants', label: 'Wants', width: 'minmax(var(--table-name-min), 1.4fr)', renderMissing: true,
      value: row => row.equipment[0]?.name ?? null, text: row => row.equipment.map(item => item.name ?? EQUIPMENT_UNNAMED).join(', '),
      render: row => <EquipmentCell name={row.equipment[0]?.name ?? null} icon={row.equipment[0] ? art.get(row.equipment[0].variant) : null} detail={firstWithMore(row.equipment)} /> },
    { key: 'amount', header: 'Amount', label: 'Amount', numeric: true, width: 'max-content', renderMissing: true,
      value: row => row.equipment[0]?.amount ?? null, render: row => units(row.equipment[0]?.amount ?? null) },
    { key: 'level', header: 'Price offered', label: 'Price offered', width: 'max-content', priority: 1, renderMissing: true,
      value: row => row.levels[0]?.level ?? null, text: row => levelText(row.levels[0]?.level ?? null), render: row => levelText(row.levels[0]?.level ?? null) },
  ], [view, country, art]);
  const offerColumns = useMemo<readonly TableColumn<OfferRow>[]>(() => [
    country('seller', 'Seller') as TableColumn<OfferRow>,
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1.4fr)', renderMissing: true,
      value: row => row.name, text: row => row.name ?? EQUIPMENT_UNNAMED, render: row => <EquipmentCell name={row.name} icon={art.get(row.variant)} /> },
    { key: 'amount', header: 'On market', label: 'On market', numeric: true, bar: 'visible', width: 'minmax(90px, max-content)',
      value: row => row.amount, render: row => <CellBar value={row.amount} maximum={offers.reduce((most, item) => item.amount > most ? item.amount : most, 1n)}>{units(row.amount)}</CellBar> },
    { key: 'level', header: 'Price level', label: 'Price level', width: 'max-content', priority: 1, renderMissing: true,
      value: row => row.level, text: row => levelText(row.level), render: row => levelText(row.level) },
  ], [country, art, offers]);
  const contractTooltip = useCallback((row: ContractRow) => {
    const lines: TooltipRow[] = [];
    for (const item of row.equipment) lines.push({ label: item.name ?? EQUIPMENT_UNNAMED, value: units(item.amount), lookupKey: item.variant });
    for (const item of row.prices) lines.push({ label: `Agreed price · ${item.name ?? EQUIPMENT_UNNAMED}`, value: cic(item.amount) });
    for (const item of row.levels) lines.push({ label: 'Price level', value: levelText(item.level) });
    if (row.assigned !== null) lines.push({ label: 'Convoys assigned', value: `${row.assigned} of ${row.convoys ?? '—'}` });
    if (row.collected !== null) lines.push({ label: 'Payment collected', value: cic(row.collected) });
    if (row.deliveries !== null) lines.push({ label: 'Completed deliveries', value: row.deliveries.toString() });
    return fitTooltip([{ title: `${name(row.seller)} → ${name(row.buyer)}`, subtitle: dayLabel(hour), rows: lines }], viewport);
  }, [name, hour, viewport]);
  const offerTooltip = useCallback((row: OfferRow) => fitTooltip([{ title: row.name ?? EQUIPMENT_UNNAMED, subtitle: `${name(row.seller)} · ${dayLabel(hour)}`,
    rows: [{ label: 'On market', value: units(row.amount) }, { label: 'Price level', value: levelText(row.level) }] }], viewport), [name, hour, viewport]);
  const playing = runtime.clock.getSnapshot().playing;
  const open = (title: string, rowTags: (string | null)[], selected: string | null) => {
    setPicked({ title, tags: rowTags.filter((tag): tag is string => tag !== null), selected });
    dispatch({ type: 'open', breakdown: DETAILS.id, hour, playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const count = view === 'inventory' ? offers.length : contracts.length;
  const at = drillHour(drill, hour) ?? hour;

  if (!track) return <WidgetState kind="empty" icon="globe" message="Load a campaign to see the market." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (needsGroups && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  const header = headerSlot && createPortal(<>
    <Segmented value={view} options={VIEWS} onChange={value => setOption?.('view', value)} label="Market view" />
    {ready && <span className="pill">{count.toLocaleString()} {view === 'contracts' ? 'active contracts' : view === 'requests' ? 'open requests' : 'listed'}</span>}
  </>, headerSlot);
  if (!ready) return <>{header}<WidgetState kind="loading" label="Loading the market…" /></>;
  if (!preparedAt(data!, hour)) return <>{header}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  // A group whose membership at this hour is unrecorded or empty says so, never "none".
  const groupReason = groupReasonAt(widget.countries, groups.data, hour, track.branch.lastHour);
  if (groupReason) return <>{header}<WidgetState kind={groupReason.kind} message={groupReason.message} /></>;
  const common = { sort, setOption: (key: string, value: Parameters<NonNullable<WidgetProps['setOption']>>[1]) => setOption?.(key === 'tableSort' ? `sort-${view}` : key, value),
    cellBars: options.cellBars === true, density: (options.density === 'compact' ? 'compact' : 'comfortable') as 'compact' | 'comfortable',
    rowHeight: 2 * (tokens.fontSize.title + tokens.fontSize.small), narrow: narrowTableOf(options), hoursPerSecond };
  const body = !knownAt(data!, hour)
    ? <WidgetState kind="unavailable" message={view === 'inventory' ? 'Market inventory was not recorded.' : 'Contract terms unavailable.'} />
    : view === 'inventory'
      ? offers.length ? <Table key={`${track.source.id}:${track.branch.id}:inventory`} label="Listed equipment" rows={offers} columns={offerColumns} rowKey={row => row.key}
        tooltip={offerTooltip} onVisibleRows={onVisibleOffers} onRowClick={row => open(`${name(row.seller)}: market`, [row.seller], null)} {...common} />
        : <WidgetState kind="empty" icon="globe" message="Nothing listed." />
      : contracts.length ? <Table key={`${track.source.id}:${track.branch.id}:${view}`} label={view === 'contracts' ? 'Market contracts' : 'Market requests'}
        rows={contracts} columns={contractColumns} rowKey={row => row.key} tooltip={contractTooltip} onVisibleRows={onVisibleContracts}
        onRowClick={row => open(`${name(row.seller)} → ${name(row.buyer)}`, [row.seller, row.buyer], `contract:${view === 'requests'}:${row.contract.id}`)} {...common} />
        : <WidgetState kind="empty" icon="globe" message={view === 'contracts' ? 'No market contracts at this date.' : 'No open requests.'} />;
  return <div ref={anchor} className="international-market">
    {header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked} anchor={anchor}
      title={picked?.title ?? DETAILS.label} day={dayLabel(at)} following={drill.follow} canFollow={widget.time.mode === 'follow'}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={<div className="international-market-table">{body}</div>}>
      {picked && <MarketDrill runtime={runtime} track={track} hour={drill.follow ? hour : at} tags={picked.tags} selected={picked.selected} active={active} />}
    </DrillHost>
  </div>;
}
