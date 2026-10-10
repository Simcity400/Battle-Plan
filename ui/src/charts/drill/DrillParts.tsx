import { useCallback, useEffect, useMemo, useRef } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MovingRow } from '../rowMotion';
import WindowedRows from '../WindowedRows';
import { drillPartShare, drillPartsTotal, orderedDrillParts } from './parts';
import type { DrillPart } from './parts';
import '../charts.css';
import './drill.css';

/** One part: its value (for the bar and share), an exact text for it if the widget
 * formats one, and its colour in the widget's chart (shown as a swatch too). */
export type { DrillPart } from './parts';

/** The parts of a value at one moment, largest first, each with its amount and its
 * share of the total. Labels come from the game or the recording only; whatever the
 * recording cannot attribute is shown as its own row, never folded into another. */
export default function DrillParts({ parts, format, total, unattributed, totalRow, showShares = true, virtual = false,
  positiveBars = false, signedShares = false, active = true, onVisible, onPick }: {
  parts: readonly DrillPart[]; format: (value: number) => string; total?: number | null;
  /** Incomplete totals cannot supply a percentage. */
  showShares?: boolean;
  /** Full inventories use the existing viewport rows, with no data truncation. */
  virtual?: boolean;
  active?: boolean; onVisible?: (ids: readonly string[]) => void;
  /** Native row activation is opt-in; residual and refused IDs are never picked. */
  onPick?: (id: string) => void;
  /** Corrections retain selectable exact rows, without positive amount geometry. */
  positiveBars?: boolean; signedShares?: boolean;
  /** The part of the total no recorded source explains, with the game's own label if it has one. */
  unattributed?: { label: string; value: number; raw?: bigint | null; text?: string; shareText?: string; color?: string };
  /** A closing total row. */
  totalRow?: { label: string; value: string };
}) {
  const rows = useMemo(() => {
    const ordered = orderedDrillParts(parts).map(item => ({ key: JSON.stringify(['part', item.id]), item }));
    if (unattributed && (unattributed.raw === undefined ? unattributed.value !== 0 : unattributed.raw !== 0n)) ordered.push({ key: 'unattributed',
      item: { id: '', label: unattributed.label, value: unattributed.value, raw: unattributed.raw, text: unattributed.text,
        shareText: unattributed.shareText, color: unattributed.color, muted: true } });
    return ordered;
  }, [parts, unattributed]);
  const magnitude = (value: number | null) => positiveBars ? Math.max(0, value ?? 0) : Math.abs(value ?? 0);
  const widest = rows.reduce((result, row) => Math.max(result, magnitude(row.item.value)), 1e-12);
  const sum = total === undefined ? drillPartsTotal(rows.map(row => row.item)) : total;
  const reported = useRef<{ callback: typeof onVisible; ids: readonly string[] } | null>(null);
  const publishVisible = useCallback((ids: readonly string[]) => {
    const previous = reported.current;
    if (previous && previous.callback === onVisible && previous.ids.length === ids.length
      && previous.ids.every((id, index) => id === ids[index])) return;
    reported.current = { callback: onVisible, ids };
    onVisible?.(ids);
  }, [onVisible]);
  const visible = useCallback((mounted: readonly MovingRow<DrillPart>[]) => {
    if (active) publishVisible([...new Set(mounted.map(row => row.item.id))]);
  }, [active, publishVisible]);
  const visibleCallback = useRef(onVisible);
  visibleCallback.current = onVisible;
  useEffect(() => () => visibleCallback.current?.([]), []);
  useEffect(() => {
    if (!active) publishVisible([]);
    else if (!virtual) publishVisible(rows.map(row => row.item.id));
  }, [active, virtual, rows, publishVisible]);
  const selectable = (row: DrillPart) => onPick !== undefined && row.id !== '' && row.selectable !== false;
  const amount = (row: DrillPart) => row.text ?? (row.value == null ? 'Not known' : format(row.value));
  const content = (row: DrillPart) => <>
    <span className="drill-part-label">{row.countryFlag !== undefined
      ? <CountryFlagUrl className="drill-part-flag" url={row.countryFlag.flag} reason={row.countryFlag.flagReason} pending={row.countryFlag.flagPending} />
      : row.icon && <img src={row.icon} alt="" />}{row.color && <i className="drill-part-swatch" aria-hidden="true" style={{ background: row.color }} />}
      {selectable(row) ? <button type="button" tabIndex={0} className="drill-part-pick" aria-label={`${row.label}, ${amount(row)}`}
        onClick={() => onPick?.(row.id)}>{row.label}</button> : row.label}</span>
    <span className="drill-part-bar" aria-hidden="true"><i style={{ width: `${magnitude(row.value) / widest * 100}%`,
      background: row.color ?? (row.value != null && row.value < 0 ? 'var(--error)' : 'var(--positive)') }} /></span>
    <b className={row.value != null && row.value < 0 ? 'is-negative' : undefined}>{amount(row)}</b>
    <small>{showShares ? row.shareText ?? drillPartShare(row.value, sum, signedShares) : null}</small>
  </>;
  const closing = totalRow && <div className="drill-parts drill-total"><span className="drill-part-label">{totalRow.label}</span><span /><b>{totalRow.value}</b><small /></div>;
  if (!rows.length && !totalRow) return <p className="drill-empty">Nothing recorded for this day.</p>;
  if (virtual) return <div className="drill-inventory">
    <WindowedRows items={active ? rows : []} rowHeight={32} label="Recorded parts" onVisibleRows={visible}>{({ item, key }) => <div tabIndex={selectable(item) ? undefined : 0}
      className={`drill-parts drill-part-row${key === 'unattributed' ? ' is-unattributed' : item.muted || item.value == null ? ' is-muted' : ''}`}>{content(item)}</div>}</WindowedRows>
    {closing}
  </div>;
  return <ol className="drill-parts">{rows.map(({ key, item }) => <li key={key} tabIndex={selectable(item) ? undefined : 0}
    className={key === 'unattributed' ? 'is-unattributed' : item.muted || item.value == null ? 'is-muted' : undefined}>{content(item)}</li>)}
    {totalRow && <li className="is-total"><span className="drill-part-label">{totalRow.label}</span><span /><b>{totalRow.value}</b><small /></li>}
  </ol>;
}
