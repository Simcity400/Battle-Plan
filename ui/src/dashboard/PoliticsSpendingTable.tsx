import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../charts/table/options.ts';
import { ChipSet } from '../components/ChipSet';
import ArtFallback from '../components/ArtFallback';
import WidgetState from '../components/WidgetState';
import { MEMBERSHIP_PREPARING } from './groups';
import Table, { CountryCell } from '../charts/table/Table';
import type { TableColumn } from '../charts/table/Table';
import { useTableHour } from '../charts/table/useTableHour';
import { dayLabel } from '../charts/draw';
import { useChartTokens, useViewportHeight } from '../charts/useChartTokens';
import { jumpItem } from '../charts/jump';
import { fitTooltip } from '../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import type { WidgetProps } from './definitions';
import { useCountryLabelsAt } from './countryFlagsAt';
import { ideaTooltip } from './ideaTooltip';
import { resolveWindow } from './model';
import { contentSet, resolveOptions, setToken, toggleContentSet } from './options';
import { usePlaybackSpeed } from './playback';
import { categoryChoices, monthLabel, spendLabel, spendingAt, spendingCostNotes, spendingCountryRequests, spendingGapNote, spendingGaps, spendingSummary,
  preparedSpendingTableRows, unknownCosts } from './politicsSpending';
import type { PowerCurrency, SpendingTableRow } from './politicsSpending';
import { NO_SPENDING_CATEGORIES, normalizePoliticsSpendingOptions, POLITICS_SPENDING_OPTIONS, spendingSort } from './politicsSpendingOptions';
import { powerGain } from './powerBalance';
import { formatPower, formatPowerChange, formatSpent, powerDecimals } from './powerDecimals';
import { usePoliticsSpending, useSpendingIdeaTooltips } from './usePoliticsSpending';
import './politicsSpending.css';
import { useParityRows } from './parityRows';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly SpendingTableRow[] = [];
const rowKey = (row: SpendingTableRow) => row.key;
/** The chip that clears the category filter. */
const ALL = '';

/** Both spending widgets: a compact table of one country's (or several countries')
 * recorded payments, with recurring daily costs folded into one row a month, and a
 * one-line summary of what was spent and gained in the shown period. */
export default function PoliticsSpendingTable({ currency, widget, runtime, active, onDiagnostics, setOptions, setOption }: WidgetProps & { currency: PowerCurrency }) {
  const parityRoot = useRef<HTMLDivElement>(null);
  const clockHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const viewport = useViewportHeight();
  const options = resolveOptions(POLITICS_SPENDING_OPTIONS, widget.options);
  const digits = powerDecimals(options);
  const unit = currency === 'Political' ? 'PP' : 'CP';
  const noun = currency === 'Political' ? 'political power' : 'command power';
  const { track, tags, data, error, groupError, retry, ready, reason, known, selection, groupWarnings } = usePoliticsSpending(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const hour = window?.lastHour;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const spends = useMemo(() => data && hour != null ? spendingAt(data, currency, hour, from, selection) : [], [data, currency, hour, from, selection]);
  const gaps = useMemo(() => data && hour != null ? spendingGaps(data, currency, { from, hour }, selection) : [], [data, currency, hour, from, selection]);
  const gain = useMemo(() => data && hour != null ? powerGain(data, currency, { from, hour }, selection) : null, [data, currency, hour, from, selection]);
  const chips = useMemo(() => categoryChoices(data, currency), [data, currency]);
  const normalized = normalizePoliticsSpendingOptions(widget.options, data ? chips : undefined);
  const categories = normalized.categories;
  const repairSelection = widget.options?.filterMode === 'shown' && categories !== widget.options.categories;
  const migration = useRef('');
  useEffect(() => {
    const key = JSON.stringify([widget.id, widget.options?.filterMode, widget.options?.categories, categories]);
    if ((normalized.migrated || repairSelection) && setOptions && migration.current !== key) {
      migration.current = key;
      setOptions({ categories, filterMode: 'shown' });
    }
  }, [widget.id, widget.options?.filterMode, widget.options?.categories, categories, normalized.migrated, repairSelection, setOptions]);
  const chosen = useMemo(() => contentSet(categories).filter(value => value !== NO_SPENDING_CATEGORIES), [categories]);
  const rows = useMemo(() => preparedSpendingTableRows(data ?? [], spends, categories), [data, spends, categories]);
  const summary = useMemo(() => spendingSummary(spends), [spends]);
  useParityRows(parityRoot, () => ({ kind: 'table', label: `${currency === 'Political' ? 'Political' : 'Command'} power spending rows`,
    complete: !!data && ready, total: rows.length, rows: rows.map(row => ({ key: row.key, tag: row.tag, hour: row.hour, month: row.month,
      category: row.category, days: row.days, amountRaw: row.amount?.toString() ?? null, partial: row.partial, order: row.order.toString() })),
    spentRaw: summary.spent.toString(), unknownCosts: summary.unknownCosts }));
  const tokens = useChartTokens();
  const colours = useMemo(() => {
    let index = 0;
    return new Map(chips.map(chip => [chip.value, chip.identified ? tokens.categories[index++ % tokens.categories.length] : tokens.muted]));
  }, [chips, tokens.categories, tokens.muted]);
  const [visible, setVisible] = useState<readonly SpendingTableRow[]>(NO_ROWS);
  const onVisibleRows = useCallback((shown: readonly SpendingTableRow[]) => setVisible(shown), []);
  const compare = tags.length > 1;
  // Names and flags only for mounted rows, each at its own date.
  const names = useMemo(() => spendingCountryRequests(visible, compare), [visible, compare]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, names.wanted, active, names.maxHours);
  const countryName = useCallback((row: SpendingTableRow) => (row.month === null ? row.parts[0].spend.countryName : null)
    ?? labelAt(row.hour)(row.tag).name, [labelAt]);
  const ideas = useSpendingIdeaTooltips(widget, runtime, active, data);
  const artwork = options.artwork !== false;
  const icon = currency === 'Political' ? 'sparkle' : 'target';
  const columns = useMemo<readonly TableColumn<SpendingTableRow>[]>(() => [
    { key: 'date', header: 'Date', label: 'Date', width: 'max-content', priority: 1, value: row => row.order,
      render: row => row.month === null ? dayLabel(row.hour) : monthLabel(row.month) },
    ...(compare ? [{ key: 'country', header: 'Country', label: 'Country', width: 'minmax(var(--table-name-min), 1fr)',
      value: (row: SpendingTableRow) => countryName(row), render: (row: SpendingTableRow) => <CountryCell {...labelAt(row.hour)(row.tag)} name={countryName(row)} /> }] : []),
    { key: 'item', header: 'Item', label: 'Item', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.label,
      render: row => <span className="spending-item" title={row.label}>
        {artwork && (row.icon ? <img src={row.icon} alt="" /> : <ArtFallback icon={row.month === null ? icon : 'history'} className="spending-item-art" />)}
        <span>{row.label}</span>{row.days > 0 && <small>{row.days === 1 ? '1 day' : `${row.days} days`}</small>}
      </span> },
    { key: 'category', header: 'Category', label: 'Category', width: 'minmax(7em, 1fr)', priority: 2, value: row => row.categoryName,
      render: row => <span className="spending-category" title={row.categoryName}>
        <i style={{ background: colours.get(setToken(row.category)) ?? tokens.muted }} />
        <span>{row.categoryName}</span></span> },
    { key: 'amount', header: 'Amount', label: 'Amount', numeric: true, width: 'max-content', bar: 'visible', value: row => row.amount,
      render: row => <>{formatSpent(row.amount!, digits)}{row.partial && <small className="spending-partial"> partial</small>}</> },
  ], [compare, countryName, labelAt, artwork, icon, colours, tokens.muted, digits]);
  const tooltip = useCallback((row: SpendingTableRow): TooltipContent => {
    const country = labelAt(row.hour)(row.tag);
    const countryRow: TooltipRow[] = compare ? [{ label: 'Country', value: countryName(row), countryFlag: country }] : [];
    if (row.month !== null) {
      const byItem = new Map<string, bigint>();
      for (const at of row.parts) {
        const name = at.item.name ?? spendLabel(at.item, row.categoryName);
        byItem.set(name, (byItem.get(name) ?? 0n) + at.recordedCharge);
      }
      const items = [...byItem].sort((a, b) => a[1] === b[1] ? a[0].localeCompare(b[0]) : a[1] > b[1] ? -1 : 1);
      const named = items.some(([label]) => label !== row.categoryName);
      return fitTooltip([{ title: row.categoryName, subtitle: `${monthLabel(row.month)} · ${row.days === 1 ? '1 day' : `${row.days} days`}`, rows: [
        ...countryRow,
        // What the cost was, when that says more than its category: a modifier, each organisation.
        ...(named ? items.map(([label, amount]) => ({ label, value: formatPower(amount, digits) })) : []),
        { label: 'Spent', value: row.amount === null ? 'Not recorded' : `${formatPower(row.amount, digits)} ${unit}`, strong: true, divider: named || compare },
      ], note: row.partial ? 'Some costs weren’t recorded.' : undefined }], viewport, { keep: 1 });
    }
    const at = row.parts[0];
    const details: TooltipRow[] = [...countryRow,
      { label: 'Cost', value: row.amount === null ? 'Not recorded' : `${formatPower(row.amount, digits)} ${unit}`, tone: row.amount === null ? 'muted' : undefined }];
    const missing = unknownCosts(at);
    if (missing && at.knownCharges) details.push({ label: 'Costs not recorded', value: String(missing), tone: 'muted' });
    if (at.occurrences > 1) details.push({ label: 'Payments', value: String(at.occurrences) });
    if (at.item.targetTag != null) {
      const target = labelAt(at.spend.hour)(at.item.targetTag);
      details.push({ label: 'Target', value: at.item.targetName ?? target.name, countryFlag: target });
    }
    if (at.realized !== null && at.charge !== null && at.realized !== -at.charge) details.push({ label: 'Balance change', value: formatPowerChange(at.realized, digits) });
    const note = !at.covered ? 'Some payments may be missing.' : undefined;
    const head = { title: row.label, subtitle: `${row.categoryName} · ${dayLabel(at.spend.hour)}`, icon: row.icon };
    if (at.item.tooltipToken && ideas) {
      const info = ideas.infos.get(at.item.tooltipToken);
      const description = ideas.art.get(at.item.tooltipToken)?.description;
      return fitTooltip([ideaTooltip(head, info, ideas.sources, description, details, note, at.spend.hour, Infinity),
        ideaTooltip(head, info, ideas.sources, description, details, note, at.spend.hour)], viewport);
    }
    return fitTooltip([{ ...head, icon: head.icon ?? undefined, rows: details, note }], viewport);
  }, [labelAt, compare, countryName, digits, unit, ideas, viewport]);
  const warnings = useMemo(() => [...(data?.flatMap(reply => reply.warnings) ?? []), ...groupWarnings], [data, groupWarnings]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon={icon} message="Load a campaign and choose a country to see its spending." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  const groupNotice = groupError && <WidgetState kind="error" detail={groupError} onRetry={retry} />;
  if (!ready) return <>{groupNotice}<WidgetState kind="loading" label="Loading group membership…" /></>;
  if (reason) return <>{groupNotice}<WidgetState kind="empty" message="Selected country unavailable at this time." /></>;
  // A group's members at hours still being prepared are not known yet: no country, or nothing spent, is not proof of none.
  const preparing = <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (!tags.length) return <>{groupNotice}{known(track.branch.firstHour, track.branch.lastHour) ? <WidgetState kind="empty" message="Choose a country to see its spending." /> : preparing}</>;
  if (!data) return <>{groupNotice}<WidgetState kind="loading" label={`Loading ${noun} spending…`} /></>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{groupNotice}<WidgetState kind="unavailable" message="This time is outside the recording." /></>;
  if (data.every(reply => !reply.spendingCoverage.length)) return <>{groupNotice}<WidgetState kind="unavailable" message={`${noun[0].toUpperCase()}${noun.slice(1)} spending wasn’t recorded.`} /></>;

  const shownSpent = chosen.length ? rows.reduce((sum, row) => sum + (row.amount ?? 0n), 0n) : null;
  const notes = [spendingGapNote(gaps), ...spendingCostNotes(summary), ...(gain && !gain.complete ? ['Gain wasn’t recorded on some days.'] : [])].filter(Boolean);
  return <div className="politics-spending" ref={parityRoot}>{groupNotice}
    {options.chips !== false && chips.length > 0 && <div className="politics-spending-chips"><ChipSet label="Shown spending categories" max={Number.MAX_SAFE_INTEGER}
      selected={chosen.length ? chosen : [ALL]} chips={[{ value: ALL, label: 'All categories' }, ...chips.map(chip => ({ ...chip, colour: colours.get(chip.value) }))]}
      onToggle={value => {
        const next = value === ALL ? '' : toggleContentSet(chosen.join(' '), value, Number.MAX_SAFE_INTEGER);
        if (next != null) setOptions?.({ categories: next, filterMode: 'shown' });
      }} /></div>}
    {options.totals !== false && <p className="politics-spending-summary" aria-label="Summary">
      <span>Spent <b>{formatPower(summary.spent, digits)} {unit}</b></span>
      {shownSpent !== null && <span>Shown <b>{formatPower(shownSpent, digits)} {unit}</b></span>}
      {gain && <span>Gained <b>{formatPower(gain.gained, digits)} {unit}</b></span>}
      {gain && gain.lost > 0n && <span>Lost at the cap <b>{formatPower(gain.lost, digits)} {unit}</b></span>}
    </p>}
    {notes.length > 0 && <p className="politics-spending-note">{notes.join(' ')}</p>}
    {rows.length ? <div className="politics-spending-table">
      <Table key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label={currency === 'Political' ? 'Political power spending' : 'Command power spending'}
        rows={rows} columns={columns} rowKey={rowKey} sort={spendingSort(widget.options)} setOption={setOption}
        cellBars={options.cellBars !== false} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        tooltip={tooltip} menu={row => [jumpItem(runtime, track.id, row.hour)]} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />
    </div> : !known(from ?? track.branch.firstHour, hour) ? preparing
      : <p className="politics-spending-empty">{chosen.length ? 'Nothing spent in these categories in this period.' : `No ${noun} spent in this period.`}</p>}
  </div>;
}
