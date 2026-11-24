import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { narrowTableOf } from '../charts/table/options.ts';
import { ChipSet } from '../components/ChipSet';
import ArtFallback from '../components/ArtFallback';
import WidgetState from '../components/WidgetState';
import Table, { CountryCell } from '../charts/table/Table';
import type { TableColumn } from '../charts/table/Table';
import type { SortValue } from '../charts/table/logic';
import { dayLabel } from '../charts/draw';
import { useChartTokens, useViewportHeight } from '../charts/useChartTokens';
import { jumpItem } from '../charts/jump';
import { fitTooltip } from '../charts/tooltip';
import type { TooltipContent, TooltipRow } from '../charts/tooltip';
import { liveAt } from '../charts/live.ts';
import type { Live, LiveValues } from '../charts/live';
import LiveText, { liveTexts } from '../charts/LiveText';
import { aheadEnd, useRevealClock } from '../charts/reveal.ts';
import { useAheadHour } from './aheadHour.ts';
import type { WidgetProps } from './definitions';
import { useCountryLabelsAt } from './countryFlagsAt';
import { nameKey } from './countryFacts.ts';
import { ideaTooltip } from './ideaTooltip';
import { useIdeaTooltips } from './useIdeaTooltips';
import { resolveWindow } from './model';
import { contentSet, resolveOptions, setToken, toggleContentSet } from './options';
import { usePlaybackMotion } from './playback';
import { categoryChoices, groupSummary, mergedIdeaTooltips, monthFigures, monthItems, monthLabel, namesCovered, powerGains, spendingCostNotes, spendingCountryRequests,
  spendingGapNote, spendingGaps, spendingGroups, spendingRowsAhead, spendingTableRows, stableSpendingRows } from './powerSpending.ts';
import type { PowerCurrency, SpendingTableRow, SpendingWindow } from './powerSpending';
import { NO_SPENDING_CATEGORIES, normalizePowerSpendingOptions, POWER_SPENDING_OPTIONS, spendingSort } from './powerSpendingOptions';
import { formatPower, formatSpent, powerDecimals } from './powerDecimals';
import { structureAt, structureParts, usePowerSpending } from './usePowerSpending.ts';
import { useShownHour } from './widgetPerf.ts';
import './powerSpending.css';
import ReadView from './ReadView';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly SpendingTableRow[] = [];
const rowKey = (row: SpendingTableRow) => row.key;
const ALL = '';
const NOT_RECORDED = 'Not recorded';
const MOUNTED_SLACK = 64;
const requestsKey = (requests: { wanted: readonly { hour: number; tag: string }[]; maxHours: number }) =>
  `${requests.maxHours}|${requests.wanted.map(item => `${item.hour}\u0000${item.tag}`).sort().join('\u0001')}`;

export default function PowerSpendingTable({ currency, widget, runtime, active, onDiagnostics, setOptions, setOption }: WidgetProps & { currency: PowerCurrency }) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const viewport = useViewportHeight();
  const options = resolveOptions(POWER_SPENDING_OPTIONS, widget.options);
  const digits = powerDecimals(options);
  const unit = currency === 'Political' ? 'PP' : 'CP';
  const noun = currency === 'Political' ? 'political power' : 'command power';
  const { track, read, tags, ledgers, error, groupError, retry, ready, reason, selection, groupWarnings } = usePowerSpending(widget, runtime, active, currency);
  const following = widget.time.mode === 'follow';
  const from = widget.time.mode === 'range' && track ? resolveWindow(widget, track, runtime.tracks, 0)?.firstHour ?? null : null;
  const hourAt = useCallback((clockHour: number) => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null,
    [widget, track, runtime.tracks]);
  const structure = useMemo(() => ledgers ? structureParts(ledgers, selection) : null, [ledgers, selection]);
  const end = useCallback((clockHour: number) => {
    const at = hourAt(clockHour);
    if (at === null || !structure) return clockHour + 1;
    const { hoursPerSecond, playing } = runtime.clock.getSnapshot();
    const next = structure.other[structureAt(structure.other, at)] ?? Infinity;
    return Math.min(aheadEnd(structure.payments, at, hoursPerSecond, playing), next) - (at - clockHour);
  }, [hourAt, structure, runtime.clock]);
  const block = useAheadHour(runtime.clock, active && following && !!ledgers, end);
  const hour = following ? hourAt(block.hour) : hourAt(runtime.clock.getHour());
  const drawnHour = following && hour !== null && Number.isFinite(block.until) ? Math.max(hour, hour + block.until - 1 - block.hour) : hour;
  const drawn = useMemo<SpendingWindow | null>(() => drawnHour === null ? null : { from, hour: drawnHour }, [from, drawnHour]);
  const chips = useMemo(() => categoryChoices(ledgers), [ledgers]);
  const normalized = normalizePowerSpendingOptions(widget.options, ledgers ? chips : undefined);
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
  const heldRows = useRef<ReadonlyMap<string, SpendingTableRow>>(new Map());
  const rows = useMemo(() => {
    if (!ledgers || !drawn) return NO_ROWS;
    const made = stableSpendingRows(heldRows.current, spendingTableRows(ledgers, drawn, categories, selection));
    heldRows.current = new Map(made.map(row => [row.key, row]));
    return made;
  }, [ledgers, drawn, categories, selection]);
  const trackHour = useCallback((clockHour: number) => hourAt(clockHour) ?? Number.NaN, [hourAt]);
  const reveal = useRevealClock(runtime.clock, following, trackHour);
  const tableReveal = useMemo(() => reveal ? { clock: reveal, at: (row: SpendingTableRow) => row.payment?.read ?? -Infinity } : undefined, [reveal]);
  const byKey = useMemo(() => new Map(rows.map(row => [row.key, row])), [rows]);
  const groups = useMemo(() => ledgers ? spendingGroups(ledgers, from, selection) : null, [ledgers, from, selection]);
  const gains = useMemo(() => ledgers ? powerGains(ledgers, from, selection) : null, [ledgers, from, selection]);
  const shown = useShownHour();
  const paintedHour = useCallback(() => {
    const at = following ? hourAt(runtime.clock.getHour()) : hour;
    return at === null ? Number.NaN : at;
  }, [following, hourAt, runtime.clock, hour]);
  useEffect(() => { if (!following) shown(ledgers ? hour : null); }, [following, ledgers, hour, shown]);
  const live = useMemo<Live<LiveValues<SortValue>>>(() => liveAt(runtime.clock, position => {
    const at = following ? hourAt(position) : hour;
    return at === null ? Number.NaN : at;
  }, at => {
    if (following && ledgers) shown(Number.isNaN(at) ? null : at);
    return { get: key => {
      const row = byKey.get(key.slice(0, key.lastIndexOf('\u0000')));
      if (!row) return undefined;
      const amount = row.month === null ? row.amount : monthFigures(row, at, from).amount;
      return amount ?? undefined;
    }, missing: () => NOT_RECORDED };
  }), [runtime.clock, following, hourAt, hour, byKey, from, ledgers, shown]);
  const texts = useMemo(() => liveTexts(runtime.clock, paintedHour, at => {
    const values = new Map<string, string>();
    if (!ledgers || !groups || !gains || Number.isNaN(at)) return values;
    const summary = groupSummary(groups, at);
    values.set('spent', `${formatPower(summary.spent, digits)} ${unit}`);
    values.set('notes', [spendingGapNote(spendingGaps(ledgers, { from, hour: at }, selection)), ...spendingCostNotes(summary)].filter(Boolean).join(' '));
    const gain = gains(at);
    values.set('gained', gain ? `${formatPower(gain.gained, digits)} ${unit}` : NOT_RECORDED);
    values.set('lost', gain && gain.lost > 0n ? `Lost at the cap ${formatPower(gain.lost, digits)} ${unit}` : '');
    values.set('gainNote', gain && !gain.complete ? 'Gain wasn’t recorded on some days.' : '');
    if (chosen.length) {
      let shownSpent = 0n;
      for (const row of rows) if (!row.payment || row.payment.read <= at) shownSpent += (row.month === null ? row.amount : monthFigures(row, at, from).amount) ?? 0n;
      values.set('shown', `${formatPower(shownSpent, digits)} ${unit}`);
    }
    for (const row of rows) if (row.month !== null) {
      const days = monthFigures(row, at, from).days;
      values.set(row.key, days === 1 ? '1 day' : `${days} days`);
    }
    return values;
  }), [runtime.clock, paintedHour, ledgers, groups, gains, from, selection, digits, unit, chosen.length, rows]);
  const tokens = useChartTokens();
  const colours = useMemo(() => {
    let index = 0;
    return new Map(chips.map(chip => [chip.value, chip.identified ? tokens.categories[index++ % tokens.categories.length] : tokens.muted]));
  }, [chips, tokens.categories, tokens.muted]);
  const compare = tags.length > 1;
  const [mounted, setMounted] = useState(() => ({ ...spendingCountryRequests(NO_ROWS, compare), through: -Infinity }));
  const since = useMemo(() => following && drawnHour !== null ? spendingRowsAhead(rows, mounted.through + 1, drawnHour) : NO_ROWS,
    [following, drawnHour, rows, mounted.through]);
  const asked = useMemo(() => {
    const names = since.length ? spendingCountryRequests(NO_ROWS, compare, since) : null;
    if (!names?.wanted.length || namesCovered(mounted.wanted, names.wanted)) return mounted;
    const wanted = [...new Map([...mounted.wanted, ...names.wanted].map(item => [nameKey(item.hour, item.tag), item])).values()];
    return { wanted, maxHours: Math.max(mounted.maxHours, names.maxHours, new Set(wanted.map(item => item.hour)).size) };
  }, [mounted, since, compare]);
  const named = useRef({ rows: NO_ROWS, compare, key: requestsKey(mounted), asked: asked.wanted, drawn: drawnHour });
  named.current.asked = asked.wanted;
  named.current.drawn = drawnHour;
  const onVisibleRows = useCallback((drawn: readonly SpendingTableRow[]) => {
    const held = named.current;
    const next = spendingCountryRequests(drawn, held.compare), key = requestsKey(next);
    held.rows = drawn;
    if (key === held.key || (namesCovered(held.asked, next.wanted) && held.asked.length <= 2 * next.wanted.length + MOUNTED_SLACK)) return;
    held.key = key;
    setMounted({ ...next, through: held.drawn ?? -Infinity });
  }, []);
  useEffect(() => {
    if (named.current.compare === compare) return;
    named.current.compare = compare;
    onVisibleRows(named.current.rows);
  }, [compare, onVisibleRows]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, asked.wanted, active, asked.maxHours);
  const countryName = useCallback((row: SpendingTableRow) => labelAt(row.hour)(row.tag).name, [labelAt]);
  const ideaData = useMemo(() => mergedIdeaTooltips(ledgers), [ledgers]);
  const ideas = useIdeaTooltips(ideaData, track, runtime.client, active, tag => tag);
  const artwork = options.artwork !== false;
  const icon = currency === 'Political' ? 'sparkle' : 'target';
  const iconOf = useCallback((row: SpendingTableRow) => {
    const key = row.icon;
    return key ? ledgers?.find(ledger => ledger.tag === row.tag)?.icons.get(key) ?? null : null;
  }, [ledgers]);
  const countryColumn = useMemo<TableColumn<SpendingTableRow> | null>(() => compare ? { key: 'country', header: 'Country', label: 'Country',
    width: 'minmax(var(--table-name-min), 1fr)', value: row => countryName(row),
    render: row => <CountryCell {...labelAt(row.hour)(row.tag)} name={countryName(row)} /> } : null, [compare, countryName, labelAt]);
  const columns = useMemo<readonly TableColumn<SpendingTableRow>[]>(() => [
    { key: 'date', header: 'Date', label: 'Date', width: 'max-content', priority: 1, value: row => row.order,
      render: row => row.month === null ? dayLabel(row.hour) : monthLabel(row.month) },
    ...(countryColumn ? [countryColumn] : []),
    { key: 'item', header: 'Item', label: 'Item', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.label,
      render: row => {
        const image = iconOf(row);
        return <span className="spending-item" title={row.label}>
          {artwork && (image ? <img src={image} alt="" /> : <ArtFallback icon={row.month === null || row.icon ? icon : 'history'} className="spending-item-art" />)}
          <span>{row.label}</span>{row.month !== null && <small><LiveText live={texts} at={row.key} /></small>}
        </span>;
      } },
    { key: 'category', header: 'Category', label: 'Category', width: 'minmax(7em, 1fr)', priority: 2, value: row => row.categoryName,
      render: row => <span className="spending-category" title={row.categoryName}>
        <i style={{ background: colours.get(setToken(row.category)) ?? tokens.muted }} />
        <span>{row.categoryName}</span></span> },
    { key: 'amount', header: 'Amount', label: 'Amount', numeric: true, width: 'max-content', bar: 'visible', value: row => row.amount,
      paint: value => typeof value === 'bigint' ? formatSpent(value, digits) : String(value),
      render: row => <>{row.amount === null ? NOT_RECORDED : formatSpent(row.amount, digits)}{row.partial && <small className="spending-partial"> partial</small>}</> },
  ], [countryColumn, artwork, icon, colours, tokens.muted, digits, iconOf, texts]);
  const tooltip = useCallback((row: SpendingTableRow): TooltipContent => {
    const country = labelAt(row.hour)(row.tag);
    const countryRow: TooltipRow[] = compare ? [{ label: 'Country', value: countryName(row), countryFlag: country }] : [];
    const at = paintedHour();
    if (row.month !== null) {
      const now = monthFigures(row, at, from);
      const items = monthItems(row, at, from);
      const named = items.some(([label]) => label !== row.categoryName);
      return fitTooltip([{ title: row.categoryName, subtitle: `${monthLabel(row.month)} · ${now.days === 1 ? '1 day' : `${now.days} days`}`, rows: [
        ...countryRow,
        ...(named ? items.map(([label, amount]) => ({ label, value: formatPower(amount, digits) })) : []),
        { label: 'Spent', value: now.amount === null ? NOT_RECORDED : `${formatPower(now.amount, digits)} ${unit}`, strong: true, divider: named || compare },
      ], note: now.partial ? 'Some costs weren’t recorded.' : undefined }], viewport, { keep: 1 });
    }
    const entry = row.payment!;
    const details: TooltipRow[] = [...countryRow,
      { label: 'Cost', value: entry.known === null ? NOT_RECORDED : `${formatPower(entry.known, digits)} ${unit}`, tone: entry.known === null ? 'muted' : undefined }];
    if (entry.unknown && entry.known !== null) details.push({ label: 'Costs not recorded', value: String(entry.unknown), tone: 'muted' });
    if (entry.payments > 1) details.push({ label: 'Payments', value: String(entry.payments) });
    if (entry.item.targetTag !== null) {
      const target = labelAt(row.hour)(entry.item.targetTag);
      details.push({ label: 'Target', value: entry.item.targetName ?? target.name, countryFlag: target });
    }
    if (entry.item.unknown) details.push({ label: 'Not identified', value: entry.item.unknown, tone: 'muted' });
    if (entry.read !== entry.hour) details.push({ label: 'Read', value: dayLabel(entry.read), tone: 'muted' });
    const head = { title: row.label, subtitle: `${row.categoryName} · ${dayLabel(entry.hour)}`, icon: iconOf(row) ?? undefined };
    const note = entry.uncovered ? 'Some payments may be missing.' : undefined;
    const token = entry.item.tooltipToken;
    if (token && ideas) {
      const info = ideas.infos.get(token);
      const description = ideas.art.get(token)?.description;
      return fitTooltip([ideaTooltip(head, info, ideas.sources, description, details, note, entry.hour, Infinity),
        ideaTooltip(head, info, ideas.sources, description, details, note, entry.hour)], viewport);
    }
    return fitTooltip([{ ...head, rows: details, note }], viewport);
  }, [labelAt, compare, countryName, digits, unit, viewport, paintedHour, from, iconOf, ideas]);
  const warnings = useMemo(() => [...(ledgers?.flatMap(ledger => ledger.warnings) ?? []), ...groupWarnings], [ledgers, groupWarnings]);
  useEffect(() => { onDiagnostics(warnings); return () => onDiagnostics(NO_WARNINGS); }, [warnings, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon={icon} message="Load a campaign and choose a country to see its spending." />;
  if (read.state === 'outside' || read.state === 'refused') return <ReadView read={read} loading="">{() => null}</ReadView>;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  const groupNotice = groupError && <WidgetState kind="error" failure={groupError} onRetry={retry} />;
  if (!ready) return <>{groupNotice}<WidgetState kind="loading" label="Loading group membership…" /></>;
  if (reason) return <>{groupNotice}<WidgetState kind="empty" message="Selected country unavailable at this time." /></>;
  if (!tags.length) return <>{groupNotice}<WidgetState kind="empty" message="Choose a country to see its spending." /></>;
  if (!ledgers) return <>{groupNotice}<WidgetState kind="loading" label={`Loading ${noun} spending…`} /></>;
  if (hour === null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <>{groupNotice}<WidgetState kind="outside" /></>;
  if (ledgers.every(ledger => !ledger.coverage.length)) return <>{groupNotice}<WidgetState kind="unavailable" message={`${noun[0].toUpperCase()}${noun.slice(1)} spending wasn’t recorded.`} /></>;

  return <div className="politics-spending">{groupNotice}
    {options.chips !== false && chips.length > 0 && <div className="politics-spending-chips"><ChipSet label="Shown spending categories" max={Number.MAX_SAFE_INTEGER}
      selected={chosen.length ? chosen : [ALL]} chips={[{ value: ALL, label: 'All categories' }, ...chips.map(chip => ({ ...chip, colour: colours.get(chip.value) }))]}
      onToggle={value => {
        const next = value === ALL ? '' : toggleContentSet(chosen.join(' '), value, Number.MAX_SAFE_INTEGER);
        if (next != null) setOptions?.({ categories: next, filterMode: 'shown' });
      }} /></div>}
    {options.totals !== false && <p className="politics-spending-summary" aria-label="Summary">
      <span>Spent <b><LiveText live={texts} at="spent" /></b></span>
      {chosen.length > 0 && <span>Shown <b><LiveText live={texts} at="shown" /></b></span>}
      <span>Gained <b><LiveText live={texts} at="gained" /></b></span>
      <span><LiveText live={texts} at="lost" /></span>
    </p>}
    <p className="politics-spending-note"><LiveText live={texts} at="notes" /> {options.totals !== false && <LiveText live={texts} at="gainNote" />}</p>
    {rows.length ? <div className="politics-spending-table">
      <Table rowRevision={row => [row, columns]} key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label={currency === 'Political' ? 'Political power spending' : 'Command power spending'}
        rows={rows} columns={columns} rowKey={rowKey} sort={spendingSort(widget.options)} setOption={setOption} live={live}
        cellBars={options.cellBars !== false} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
        tooltip={tooltip} menu={row => [jumpItem(runtime, track.id, row.hour)]} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} reveal={tableReveal} />
    </div> : <p className="politics-spending-empty">{chosen.length ? 'Nothing spent in these categories in this period.' : `No ${noun} spent in this period.`}</p>}
  </div>;
}

