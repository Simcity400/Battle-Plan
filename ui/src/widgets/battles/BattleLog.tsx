import { useCallback, useMemo } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MenuItem } from '../../components/ContextMenu';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import type { LogSettings } from '../../charts/log/logOptions';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import type { Sort } from '../../charts/table/logic';
import { fitTooltip, tooltipText } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import type { WidgetProps } from '../../dashboard/definitions';
import type { BattleCountry, BattleSide, BattleValue } from '../../generated/dashboard';
import { battleDate, battleDateMoment, battleShortDate, battleLogRows, battleTooltip, battleValueText, battleWinner } from './battleRows.ts';
import type { BattleCountryLabel, BattleLabels, BattleLossFormatter, BattleMeasure, BattleRow, BattleSideSummary } from './battleRows';
import BattleSides from './BattleSides';
import './battles.css';

const rowKey = (row: BattleRow) => row.id;

export interface BattleLogProps<T extends BattleRow> {
  rows: readonly T[];
  label: string;
  labels: BattleLabels;
  country: BattleCountryLabel;
  live: boolean;
  view: 'log' | 'biggest';
  log: LogSettings;
  measures: readonly BattleMeasure<T>[];
  resultMeasure?: BattleMeasure<T>;
  sort: Sort;
  setOption?: WidgetProps['setOption'];
  cellBars: boolean;
  rankMarkers: boolean;
  hoursPerSecond: number;
  menu: (row: T) => MenuItem[];
  empty: string;
  note?: string | ((row: T) => string);
  formatLoss?: BattleLossFormatter<T>;
  compactSides?: BattleSideSummary;
  description?: (row: T) => string | null;
  tooltipRows?: (row: T) => readonly TooltipRow[];
  onVisibleRows: (rows: readonly T[]) => void;
}

function Side<T extends BattleRow>({ row, index, country, formatLoss }: { row: T; index: number; country: BattleCountryLabel; formatLoss?: BattleLossFormatter<T> }) {
  const side = row.sides[index];
  return <span className="battle-side"><b>{side.label}</b>{' '}
    {side.countries.map((part: BattleCountry) => { const named = country(part.tag, part.hour);
      return <span className="battle-country" key={JSON.stringify([part.hour, part.tag])} title={named.name}>
        <CountryFlagUrl className="log-flag" url={named.flag} reason={named.flagReason} pending={named.flagPending} />
      </span>; })}
    {side.countriesUnknown && <span className="battle-muted">{side.countriesUnknown}</span>}
    {side.counts.map((value: BattleValue) => <span className="battle-count" key={value.label}>{battleValueText(value)} {value.label}</span>)}
    {side.losses.map((value: BattleValue) => <span className="battle-count" key={value.label}>{formatLoss ? formatLoss(value, row, index, false) : battleValueText(value)} {value.label}</span>)}
  </span>;
}

/** The caller supplies every measure and word; no battle-domain facts live here. */
export default function BattleLog<T extends BattleRow>({ rows, label, labels, country, live, view, log, measures, resultMeasure, sort,
  setOption, cellBars, rankMarkers, hoursPerSecond, menu, empty, note, formatLoss, compactSides, description, tooltipRows, onVisibleRows }: BattleLogProps<T>) {
  const viewportHeight = useViewportHeight();
  const logs = useMemo(() => view === 'log' ? battleLogRows(rows, live) : [], [rows, live, view]);
  const tooltip = useCallback((row: T) => {
    const content = battleTooltip(row, labels, country, measures, typeof note === 'function' ? note(row) : note, formatLoss);
    return tooltipRows ? { ...content, rows: [...content.rows, ...tooltipRows(row)] } : content;
  }, [labels, country, measures, note, formatLoss, tooltipRows]);
  const visibleLogs = useCallback((drawn: readonly LogRow<T>[]) => onVisibleRows(drawn.map(row => row.item)), [onVisibleRows]);
  const sides = (row: T) => compactSides ? <BattleSides row={row} country={country} summary={compactSides} compact={view === 'biggest'} />
    : <span className="battle-sides">{row.sides.map((_: BattleSide, index: number) => <Side key={index} row={row} index={index} country={country} formatLoss={formatLoss} />)}</span>;
  const place = (row: T, logRow: boolean) => {
    const name = logRow ? <b className="log-text" title={row.place.name ?? labels.unknownPlace}>{row.place.name ?? labels.unknownPlace}</b>
      : <span className="battle-place" title={row.place.name ?? labels.unknownPlace}>{row.place.name ?? labels.unknownPlace}</span>;
    const detail = description?.(row);
    return detail ? <span className="battle-row-place">{name}<span className="battle-row-description" title={detail}>{detail}</span></span> : name;
  };
  const columns = useMemo<readonly TableColumn<T>[]>(() => [
    { key: 'place', header: labels.place, label: labels.place, value: row => row.place.name,
      renderMissing: true, sortable: false, width: 'minmax(160px, 1fr)', render: row => place(row, false) },
    { key: 'date', header: labels.date, label: labels.date, value: row => battleDateMoment(row)?.hour ?? null, width: 'max-content', renderMissing: true,
      render: row => <span className="battle-table-date" title={battleDate(battleDateMoment(row), labels.unknownStart)}>
        {live && row.end === null && <i className="live-dot" aria-label={labels.ongoing} />}{battleShortDate(row, labels.unknownStart)}</span> },
    ...measures.map(measure => ({ key: measure.label, header: measure.title, label: measure.title, hint: measure.hint, numeric: true, width: 'max-content',
      renderMissing: true, value: (row: T) => measure.value(row).value, bar: 'visible' as const,
      render: (row: T) => { const value = measure.value(row); return <span title={(measure.tooltipFormat ?? measure.format ?? battleValueText)(value, row)}>
        {value.value === null ? '—' : (measure.format ?? battleValueText)(value, row)}</span>; } })),
    { key: 'sides', header: compactSides?.label ?? '', label: compactSides?.label ?? label, value: row => row.id, sortable: false,
      width: compactSides ? 'calc(var(--space-6) * 7)' : 'minmax(160px, 2fr)', render: sides },
    { key: 'winner', header: labels.winner, label: labels.winner, value: row => 'unknown' in row.winner ? null : battleWinner(row), width: 'max-content',
      renderMissing: true, render: row => <span title={battleWinner(row)}>{'unknown' in row.winner ? '—' : battleWinner(row)}</span> },
  ], [labels, measures, country, label, compactSides, view, live, formatLoss, description]);
  if (view === 'biggest') return <div className="battle-view"><Table label={label} rows={rows} columns={columns} rowKey={rowKey}
    sort={sort} setOption={setOption} cellBars={cellBars} rank rankMarkers={rankMarkers} density={log.density}
    menu={menu} tooltip={row => fitTooltip([tooltip(row)], viewportHeight)} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} /></div>;
  return <div className="battle-view"><LogList anchorRows rows={logs} label={label} density={log.density} months={log.months} animate={live && log.animate}
    columns="minmax(0, 1fr) auto" onVisible={visibleLogs} menu={row => menu(row.item)} empty={empty}
    describe={row => tooltipText(tooltip(row.item))} tooltip={row => [tooltip(row.item)]}
    renderRow={row => <>
      <span className={compactSides ? 'battle-log-primary' : 'battle-log-content'}>{place(row.item, true)}
        {sides(row.item)}</span>
      <span className={`battle-result${resultMeasure ? ' battle-result-with-measure' : ''}`}>{live && row.item.end === null && <span className="battle-live"><i className="live-dot" aria-hidden="true" />{labels.ongoing}</span>}
        {row.item.end && 'unknown' in row.item.end && <span>{row.item.end.unknown}</span>}
        {resultMeasure && <span>{resultMeasure.title} {(resultMeasure.format ?? battleValueText)(resultMeasure.value(row.item), row.item)}</span>}
        {(!resultMeasure || row.item.end !== null) && <span title={battleWinner(row.item)}>{'unknown' in row.item.winner ? '—' : battleWinner(row.item)}</span>}</span>
    </>} /></div>;
}
