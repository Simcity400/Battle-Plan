import { cloneElement, useCallback, useLayoutEffect, useMemo, useRef } from 'react';
import type { RefObject } from 'react';
import type { PlaybackController } from '../../dashboard/playback';
import type { NarrowTable } from '../../charts/table/options';
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
import type { RevealClock } from '../../charts/reveal';
import { battleDate, battleDateMoment, battlePlaceText, battleShortDate, battleTooltip, battleValueText, battleWinner, rowsById } from './battleRows.ts';
import type { BattleCountryLabel, BattleLabels, BattleLossFormatter, BattleMeasure, BattleRow, BattleSideSummary } from './battleRows';
import BattleSides from './BattleSides';
import './battles.css';
import type { BattleCountry, BattleSide, BattleValue } from '../../generated/dashboard';

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
  narrow?: NarrowTable;
  hoursPerSecond: number | undefined;
  menu: (row: T) => MenuItem[];
  empty: string;
  note?: string | ((row: T) => string);
  formatLoss?: BattleLossFormatter<T>;
  compactSides?: BattleSideSummary;
  description?: (row: T) => string | null;
  tooltipRows?: (row: T) => readonly TooltipRow[];
  onVisibleRows: (rows: readonly T[]) => void;
  placePending?: (row: T) => boolean;
  paintHead?: { hour: () => number | null; rowsAt: (hour: number) => ReadonlyArray<BattleRow>; clock: Pick<PlaybackController, 'subscribe' | 'subscribeFrame'> };
  keyed?: { place: (row: T) => string | null; detail?: (row: T) => string };
  logRows: readonly LogRow<T>[];
  reveal?: RevealClock;
}

function measureText<T extends BattleRow>(measure: BattleMeasure<T>, row: T): string {
  const value = measure.value(row);
  return value.value === null ? '—' : (measure.format ?? battleValueText)(value, row);
}

function useMeasurePainter<T extends BattleRow>(root: RefObject<HTMLDivElement | null>, measures: readonly BattleMeasure<T>[], paintHead: BattleLogProps<T>['paintHead']) {
  useLayoutEffect(() => {
    if (!paintHead) return undefined;
    let painted = Number.NaN;
    const paint = () => {
      const at = paintHead.hour();
      if (at === null || at === painted || !root.current) return;
      painted = at;
      const rows = rowsById(paintHead.rowsAt(at)) as ReadonlyMap<string, T>;
      for (const element of root.current.querySelectorAll<HTMLElement>('[data-battle-measure]')) {
        const [id, label] = JSON.parse(element.dataset.battleMeasure!) as [string, string];
        const row = rows.get(id), measure = measures.find(item => item.label === label);
        if (!row || !measure) continue;
        const text = measureText(measure, row);
        if (element.textContent !== text) element.textContent = text;
      }
    };
    paint();
    const frames = paintHead.clock.subscribeFrame(paint);
    const seeks = paintHead.clock.subscribe(() => { painted = Number.NaN; paint(); });
    return () => { frames(); seeks(); };
  }, [root, measures, paintHead]);
}

function sideCell<T extends BattleRow>(row: T, index: number, country: BattleCountryLabel, formatLoss: BattleLossFormatter<T> | undefined) {
  const side = row.sides[index];
  return <span className="battle-side"><b>{side.label}</b>{' '}
    {side.countries.map((part: BattleCountry) => { const named = country(part.tag, part.hour);
      return <span className="battle-country" key={part.tag} title={named.name}>
        <CountryFlagUrl className="log-flag" url={named.flag} detail={named.flagDetail} reason={named.flagReason} pending={named.flagPending} fixedSize />
      </span>; })}
    {side.countriesUnknown && <span className="battle-muted">Countries unknown</span>}
    {side.counts.map((value: BattleValue) => <span className="battle-count" key={value.label}>{battleValueText(value)} {value.label}</span>)}
    {side.losses.map((value: BattleValue) => <span className="battle-count" key={value.label}>{formatLoss ? formatLoss(value, row, index, false) : battleValueText(value)} {value.label}</span>)}
  </span>;
}

export default function BattleLog<T extends BattleRow>({ rows, label, labels, country, live, view, log, measures, resultMeasure, sort,
  setOption, cellBars, rankMarkers, narrow, hoursPerSecond, menu, empty, note, formatLoss, compactSides, description, tooltipRows, onVisibleRows, placePending, paintHead,
  logRows, reveal, keyed }: BattleLogProps<T>) {
  const viewportHeight = useViewportHeight();
  const root = useRef<HTMLDivElement>(null);
  useMeasurePainter(root, measures, paintHead);
  const tooltip = useCallback((row: T) => {
    const content = battleTooltip(row, labels, country, measures, typeof note === 'function' ? note(row) : note, formatLoss);
    return tooltipRows ? { ...content, rows: [...content.rows, ...tooltipRows(row)] } : content;
  }, [labels, country, measures, note, formatLoss, tooltipRows]);
  const visibleLogs = useCallback((drawn: readonly LogRow<T>[]) => onVisibleRows(drawn.map(row => row.item)), [onVisibleRows]);
  const sides = (row: T) => compactSides ? <BattleSides row={row} country={country} summary={compactSides} compact={view === 'biggest'} />
    : <span className="battle-sides">{row.sides.map((_: BattleSide, index: number) => cloneElement(sideCell(row, index, country, formatLoss), { key: index }))}</span>;
  const place = (row: T, logRow: boolean) => {
    const loading = row.place.name === null && placePending?.(row) === true;
    const text = battlePlaceText(row as T & { provinceId?: number | null }, labels, loading);
    const keyedPlace = row.place.name === null && keyed ? keyed.place(row) : null;
    const painted = keyedPlace !== null ? { 'data-keyed': 'place', 'data-keyed-key': keyedPlace } : {};
    const name = logRow ? <b className={`log-text${loading ? ' is-loading' : ''}`} title={text} {...painted}>{text}</b>
      : <span className={`battle-place${loading ? ' is-loading' : ''}`} title={text} {...painted}>{text}</span>;
    const detail = description?.(row);
    const paintedDetail = keyed?.detail ? { 'data-keyed': 'detail', 'data-keyed-key': keyed.detail(row) } : {};
    return detail ? <span className="battle-row-place">{name}<span className="battle-row-description" title={detail} {...paintedDetail}>{detail}</span></span> : name;
  };
  const columns = useMemo<readonly TableColumn<T>[]>(() => [
    { key: 'place', header: labels.place, label: labels.place, value: row => row.place.name,
      renderMissing: true, sortable: false, width: 'minmax(160px, 1fr)', render: row => place(row, false) },
    { key: 'date', header: labels.date, label: labels.date, value: row => battleDateMoment(row)?.hour ?? null, width: 'max-content', renderMissing: true, priority: 1,
      text: row => battleShortDate(row, labels.unknownStart),
      render: row => <span className="battle-table-date" title={battleDate(battleDateMoment(row), labels.unknownStart)}>
        {live && row.end === null && <i className="live-dot" aria-label={labels.ongoing} />}{battleShortDate(row, labels.unknownStart)}</span> },
    ...measures.map((measure, index) => ({ key: measure.label, header: measure.title, label: measure.title, hint: measure.hint, numeric: true, width: 'max-content',
      priority: 3 + index, text: (row: T) => { const value = measure.value(row); return value.value === null ? '—' : (measure.format ?? battleValueText)(value, row); },
      renderMissing: true, value: (row: T) => measure.value(row).value, bar: 'visible' as const,
      render: (row: T) => { const value = measure.value(row); return <span title={(measure.tooltipFormat ?? measure.format ?? battleValueText)(value, row)}
        data-battle-measure={JSON.stringify([row.id, measure.label])}>{measureText(measure, row)}</span>; } })),
    { key: 'sides', header: compactSides?.label ?? '', label: compactSides?.label ?? label, value: row => row.id, sortable: false,
      width: compactSides ? 'calc(var(--space-6) * 7)' : 'minmax(160px, 2fr)', render: sides },
    { key: 'winner', header: labels.winner, label: labels.winner, value: row => 'unknown' in row.winner ? null : battleWinner(row), width: 'max-content', priority: 2,
      renderMissing: true, render: row => <span title={battleWinner(row)}>{'unknown' in row.winner ? '—' : battleWinner(row)}</span> },
  ], [labels, measures, country, label, compactSides, view, live, formatLoss, description, placePending, keyed]);
  if (view === 'biggest') return <div className="battle-view" ref={root}><Table rowRevision={row => [row, columns]} label={label} rows={rows} columns={columns} rowKey={rowKey}
    sort={sort} setOption={setOption} cellBars={cellBars} rank rankMarkers={rankMarkers} narrow={narrow} density={log.density}
    menu={menu} tooltip={row => fitTooltip([tooltip(row)], viewportHeight)} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} /></div>;
  return <div className="battle-view" ref={root}><LogList anchorRows rows={logRows} label={label} density={log.density} months={log.months} animate={live && log.animate}
    playhead={reveal?.hour()} reveal={reveal} states
    columns="minmax(0, 1fr) auto" onVisible={visibleLogs} menu={row => menu(row.item)} empty={empty}
    describe={row => tooltipText(tooltip(row.item))} tooltip={row => [tooltip(row.item)]}
    renderRow={row => <>
      <span className={compactSides ? 'battle-log-primary' : 'battle-log-content'}>{place(row.item, true)}
        {sides(row.item)}</span>
      <span className={`battle-result${resultMeasure ? ' battle-result-with-measure' : ''}`}>{live && row.item.end === null && <span className="battle-live"><i className="live-dot" aria-hidden="true" />{labels.ongoing}</span>}
        {row.item.end && 'unknown' in row.item.end && <span>{row.item.end.unknown}</span>}
        {resultMeasure && <span>{resultMeasure.title} <span data-battle-measure={JSON.stringify([row.item.id, resultMeasure.label])}>{measureText(resultMeasure, row.item)}</span></span>}
        {(!resultMeasure || row.item.end !== null) && <span title={battleWinner(row.item)}>{'unknown' in row.item.winner ? '—' : battleWinner(row.item)}</span>}</span>
    </>} /></div>;
}
