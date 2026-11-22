import { countryFromKey } from '../../dashboard/countryKey.ts';
import { createPortal } from 'react-dom';
import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { liveKey } from '../../charts/live';
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
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { MARKET, partyTags, validateMarketReply } from '../../dashboard/market.ts';
import MarketDrill from '../../dashboard/MarketDrill';
import MarketEquipment from '../../dashboard/MarketEquipment';
import { nameReadText } from '../../dashboard/nameReads.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { DETAILS, OPTIONS, SORTS, VIEWS } from './definition';
import { cic, cicLabel, collectedText, contractRows, firstWithMore, levelText, offerRows, units, unitsLabel } from './marketData';
import type { ContractRow, MarketTab, OfferRow } from './marketData';
import './internationalMarket.css';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';

const NO_WARNINGS: readonly string[] = [];
const NO_CONTRACTS: ReturnType<typeof contractRows> = { rows: [], unattributed: 0 };
const NO_OFFERS: readonly OfferRow[] = [];
const UNNAMED = 'Country name unavailable';
const rowOf = (key: string) => key.slice(0, key.lastIndexOf('\u0000'));

export default function InternationalMarketWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const [drillShown, setDrillShown] = useState<number | undefined>();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const view: MarketTab = options.view === 'requests' ? 'requests' : options.view === 'inventory' ? 'inventory' : 'contracts';
  const sort = tableSortOf(options[`sort-${view}`], SORTS[view]);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => selectedCountryKeys(widget.countries, track), [widget.countries, track]);
  const tags = useMemo(() => widget.countries.kind === 'tags' || widget.countries.kind === 'selected' ? [...fallback].sort() : [], [widget.countries.kind, fallback]);
  const params = useMemo(() => track ? { countries: tags.map(countryFromKey), only: view === 'inventory' ? 'offers' : 'contracts' } : null, [track, tags, view]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), MARKET, params, { active, validate: validateMarketReply });
  const data = useMemo(() => head.data && head.at !== null ? { ...head.data, hour: head.at } : undefined, [head.data, head.at]);
  const hour = data?.hour ?? track?.branch.lastHour ?? 0;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const live = useMemo(() => reporting(head.live, () => shown(shownAt.current())), [head.live, shown]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const ready = !!data && match.ready && !match.reason;
  const listed = useMemo(() => ready && view !== 'inventory' ? contractRows(data!, view, match.matches) : NO_CONTRACTS, [ready, data, view, match]);
  const offers = useMemo(() => ready && view === 'inventory' ? offerRows(data!, match.matches) : NO_OFFERS, [ready, data, view, match]);
  const contracts = listed.rows;
  const warnings = useMemo(() => listed.unattributed
    ? [...data?.warnings ?? [], `${listed.unattributed} ${view === 'contracts' ? 'contracts' : 'requests'} whose countries were not recorded at this date may also match`]
    : data?.warnings ?? NO_WARNINGS, [listed.unattributed, data, view]);
  useDiagnostics(onDiagnostics, warnings);
  const [visibleTags, setVisibleTags] = useState<readonly string[]>([]);
  const onVisibleContracts = useCallback((rows: readonly ContractRow[]) => setVisibleTags(rows.flatMap(row => [row.seller, row.buyer].filter((tag): tag is string => tag !== null))), []);
  const onVisibleOffers = useCallback((rows: readonly OfferRow[]) => setVisibleTags(rows.flatMap(row => row.seller ? [row.seller] : [])), []);
  const wanted = useMemo(() => visibleTags.map(tag => ({ hour, tag })), [visibleTags, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const allTags = useMemo(() => [...new Set([...contracts.flatMap(row => [row.seller, row.buyer]), ...offers.map(row => row.seller)]
    .filter((tag): tag is string => tag !== null))].map(tag => ({ hour, tag })), [contracts, offers, hour]);
  const names = useCountryNamesAt(runtime.client, track, sort.column === 'seller' || sort.column === 'buyer' ? allTags : [], active);
  const name = useCallback((tag: string | null) => tag === null ? UNNAMED : nameReadText(names.get(nameKey(hour, tag))), [names, hour]);
  const label = useCallback((tag: string | null) => tag === null ? UNNAMED : labelAt(hour)(tag).name, [labelAt, hour]);
  const country = useCallback((key: 'seller' | 'buyer', header: string, priority?: number) => ({ key, header, label: header, priority,
    width: 'minmax(4em, 1fr)', renderMissing: true,
    value: (row: ContractRow | OfferRow) => { const tag = (row as ContractRow)[key] ?? null; return tag === null ? null : name(tag); },
    text: (row: ContractRow | OfferRow) => label((row as ContractRow)[key] ?? null),
    render: (row: ContractRow | OfferRow) => { const tag = (row as ContractRow)[key] ?? null; return tag === null ? UNNAMED : <CountryCell {...labelAt(hour)(tag)} />; } }), [name, label, labelAt, hour]);
  const equipmentColumn = useCallback((header: string): TableColumn<ContractRow> => ({ key: 'equipment', header, label: header, width: 'minmax(var(--table-name-min), 1.4fr)', renderMissing: true,
    value: row => row.equipment[0]?.name ?? null, text: row => row.equipment.map(item => item.name ?? 'Equipment name unavailable').join(', '),
    render: row => <MarketEquipment name={row.equipment[0]?.name ?? null} icon={row.equipment[0]?.icon ?? null} missing={row.equipment[0]?.missing} detail={firstWithMore(row.equipment)} /> }), []);
  const contractColumns = useMemo<readonly TableColumn<ContractRow>[]>(() => view === 'contracts' ? [
    country('seller', 'Seller'), country('buyer', 'Buyer'), equipmentColumn('Equipment'),
    { key: 'price', header: 'Price', label: 'Agreed price', numeric: true, width: 'max-content', priority: 1, renderMissing: true,
      value: row => row.prices[0]?.amount ?? null, text: row => row.prices.map(item => cic(item.amount)).join(', ') || '—', render: row => cicLabel(row.prices[0]?.amount ?? null) },
    { key: 'assigned', header: 'Convoys', label: 'Convoys assigned', numeric: true, width: 'max-content', priority: 3, value: () => null,
      paint: (value, key) => `${Number(value).toLocaleString()} / ${live.read().get(liveKey(rowOf(key), 'convoys'))?.toLocaleString() ?? '—'}` },
    { key: 'collected', header: 'Collected', label: 'Payment collected', numeric: true, width: 'max-content', value: () => null, paint: value => collectedText(Number(value)) },
    { key: 'deliveries', header: 'Deliveries', label: 'Completed deliveries', numeric: true, width: 'max-content', priority: 4, value: () => null, paint: value => Number(value).toLocaleString() },
  ] : [
    country('buyer', 'Buyer'), country('seller', 'Seller', 2), equipmentColumn('Wants'),
    { key: 'amount', header: 'Amount', label: 'Amount', numeric: true, width: 'max-content', renderMissing: true,
      value: row => row.equipment[0]?.amount ?? null, render: row => unitsLabel(row.equipment[0]?.amount ?? null) },
    { key: 'level', header: 'Price offered', label: 'Price offered', width: 'max-content', priority: 1, renderMissing: true,
      value: row => row.levels[0]?.level ?? null, text: row => levelText(row.levels[0]?.level ?? null), render: row => levelText(row.levels[0]?.level ?? null) },
  ], [view, country, equipmentColumn, live]);
  const offerColumns = useMemo<readonly TableColumn<OfferRow>[]>(() => [
    country('seller', 'Seller') as TableColumn<OfferRow>,
    { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1.4fr)', renderMissing: true,
      value: row => row.name, text: row => row.name ?? 'Equipment name unavailable', render: row => <MarketEquipment name={row.name} icon={row.icon} missing={row.missing} /> },
    { key: 'amount', header: 'On market', label: 'On market', numeric: true, bar: 'visible', width: 'minmax(90px, max-content)', value: row => row.amount, render: row => unitsLabel(row.amount) },
    { key: 'level', header: 'Price level', label: 'Price level', width: 'max-content', priority: 1, renderMissing: true,
      value: row => row.level, text: row => levelText(row.level), render: row => levelText(row.level) },
  ], [country]);
  const contractTip = useCallback((row: ContractRow) => contractTooltip(row, tag => tag === null ? { name: UNNAMED } : labelAt(hour)(tag), hour, viewport,
    (column: string) => live.read().get(liveKey(row.key, column)) ?? null), [labelAt, hour, viewport, live]);
  const offerTip = useCallback((row: OfferRow) => offerTooltip(row, tag => tag === null ? { name: UNNAMED } : labelAt(hour)(tag), hour, viewport), [labelAt, hour, viewport]);
  const anchor = useRef<HTMLDivElement>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const pickOwner = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : null;
  const [pick, setPick] = useState<{ owner: string | null; title: string; tags: string[]; selected: string | null } | null>(null);
  const picked = pick && pick.owner === pickOwner ? pick : null;
  const open = (title: string, rowTags: (string | null)[], selected: string | null) => {
    const tagsOf = partyTags(...rowTags);
    if (!tagsOf) return;
    setPick({ owner: pickOwner, title, tags: tagsOf, selected });
    dispatch({ type: 'open', breakdown: DETAILS.id, hour, follow: drillControl.followsAt(hour) });
  };
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillOpen = drill.breakdown !== null && !!picked;
  const drillAt = useRegionHour(runtime, drillRegion, active && drillOpen);
  const count = data && ready ? `${(view === 'inventory' ? offers.length : contracts.length).toLocaleString()}${listed.unattributed ? '+' : ''}` : null;
  const section = view === 'inventory' ? data?.offers : data?.contracts;

  if (!track) return <WidgetState kind="empty" icon="globe" message="Load a campaign to see the market." />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const parent = head.outside ? <WidgetState kind="outside" read={head} />
    : head.error && !data ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
    : groupFailure && !groups.data ? groupFailure
    : match.reason ? <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></> : null;
  if (parent && !drillOpen) return parent;
  const header = headerSlot && createPortal(<>
    <Segmented value={view} options={VIEWS} onChange={value => setOption?.('view', value)} label="Market view" />
    {count !== null && section && <span className="pill">{count} {view === 'contracts' ? 'active contracts' : view === 'requests' ? 'open requests' : 'listed'}</span>}
  </>, headerSlot);
  if (!parent && !ready && !drillOpen) return <>{groupFailure}{header}<div ref={head.root} className="international-market"><WidgetState kind="loading" label="Loading the market…" /></div></>;
  const common = { sort, setOption: (key: string, value: Parameters<NonNullable<WidgetProps['setOption']>>[1]) => setOption?.(key === 'tableSort' ? `sort-${view}` : key, value),
    cellBars: options.cellBars === true, density: (options.density === 'compact' ? 'compact' : 'comfortable') as 'compact' | 'comfortable',
    rowHeight: 2 * (tokens.fontSize.title + tokens.fontSize.small), narrow: narrowTableOf(options), hoursPerSecond };
  const body = parent ?? (!ready ? <WidgetState kind="loading" label="Loading the market…" /> : !section
    ? <WidgetState kind="unavailable" message={view === 'inventory' ? 'Market inventory was not recorded.' : 'Contract terms unavailable.'} />
    : view === 'inventory'
      ? offers.length ? <Table rowRevision={row => [row, row.seller === null ? null : labelAt(hour)(row.seller)]} key={`${track.source.id}:${track.branch.id}:inventory`} label="Listed equipment"
        rows={offers} columns={offerColumns} rowKey={row => row.key} tooltip={offerTip} onVisibleRows={onVisibleOffers}
        onRowClick={row => open(`${label(row.seller)}: market`, [row.seller], `offer:${row.offer.country}`)} {...common} />
        : <WidgetState kind="empty" icon="globe" message="Nothing listed." />
      : contracts.length ? <Table rowRevision={row => [row, view, row.seller === null ? null : labelAt(hour)(row.seller), row.buyer === null ? null : labelAt(hour)(row.buyer)]}
        key={`${track.source.id}:${track.branch.id}:${view}`} label={view === 'contracts' ? 'Market contracts' : 'Market requests'}
        rows={contracts} columns={contractColumns} rowKey={row => row.key} tooltip={contractTip} onVisibleRows={onVisibleContracts}
        onRowClick={row => open(`${label(row.seller)} → ${label(row.buyer)}`, [row.seller, row.buyer], `contract:${view === 'requests'}:${row.terms.id}`)}
        live={view === 'contracts' ? live : undefined} {...common} />
        : listed.unattributed ? <WidgetState kind="unavailable" message={`Some ${view === 'contracts' ? 'contracts' : 'requests'}' countries were not recorded at this date.`} />
          : <WidgetState kind="empty" icon="globe" message={view === 'contracts' ? 'No market contracts at this date.' : 'No open requests.'} />);
  const drillDay = drillShown ?? drillAt;
  return <div ref={anchor} className="international-market">{groupFailure}
    {header}
    <DrillHost drill={drillControl} runtime={runtime} subject={drillOpen} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={picked?.title ?? DETAILS.label} day={drillDay == null ? '' : dayLabel(drillDay)} canFollow
      onClose={() => dispatch({ type: 'close' })}
      main={<div ref={head.root} className="international-market-table">{body}</div>}>
      {picked && <MarketDrill runtime={runtime} widget={widget} time={drillTime} tags={picked.tags} selected={picked.selected} active={active} onAnswered={setDrillShown} />}
    </DrillHost>
  </div>;
}

export function contractTooltip(row: ContractRow, label: (tag: string | null) => ReturnType<CountryLabel>, hour: number, viewport: number, progress: (column: string) => number | null) {
  const seller = label(row.seller), buyer = label(row.buyer);
  const lines: TooltipRow[] = [];
  for (const item of row.equipment) lines.push({ label: item.name ?? 'Equipment name unavailable', value: units(item.amount), lookupKey: item.variant });
  for (const item of row.prices) lines.push({ label: `Agreed price · ${item.name ?? 'Equipment name unavailable'}`, value: cic(item.amount) });
  for (const item of row.levels) lines.push({ label: 'Price level', value: levelText(item.level) });
  if (row.terms.purchase) {
    const assigned = progress('assigned'), convoys = progress('convoys'), collected = progress('collected'), deliveries = progress('deliveries');
    lines.push({ label: 'Convoys assigned', value: `${assigned ?? '—'} of ${convoys ?? '—'}` });
    lines.push({ label: 'Payment collected', value: collected === null ? '—' : cic(BigInt(Math.trunc(collected))) });
    lines.push({ label: 'Completed deliveries', value: deliveries === null ? '—' : deliveries.toString() });
  }
  return fitTooltip([{ title: `${seller.name} → ${buyer.name}`, subtitle: dayLabel(hour), ...countryNameDetails(seller, buyer), rows: lines }], viewport);
}

export function offerTooltip(row: OfferRow, label: (tag: string | null) => ReturnType<CountryLabel>, hour: number, viewport: number) {
  const seller = label(row.seller);
  return fitTooltip([{ title: row.name ?? 'Equipment name unavailable', subtitle: `${seller.name} · ${dayLabel(hour)}`, ...countryNameDetails(seller),
    rows: [{ label: 'On market', value: units(row.amount) }, { label: 'Price level', value: levelText(row.level) }] }], viewport);
}
