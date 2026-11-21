import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { MovingRow } from '../rowMotion';
import type { Live } from '../live';
import WindowedRows from '../WindowedRows';
import { drillPartShare, drillPartsTotal, drillRows } from './parts';
import type { DrillPart, DrillUnattributed } from './parts';
import '../charts.css';
import './drill.css';

export type { DrillPart } from './parts';

export type LiveDrillParts = Live<{ parts: readonly DrillPart[]; unattributed?: DrillUnattributed; total?: number | null; totalText?: string }>;

export default function DrillParts({ parts, format, total, unattributed, totalRow, showShares = true, virtual = false,
  positiveBars = false, signedShares = false, active = true, onVisible, onPick, live }: {
  parts: readonly DrillPart[]; format: (value: number) => string; total?: number | null;
  showShares?: boolean;
  virtual?: boolean;
  active?: boolean; onVisible?: (ids: readonly string[]) => void;
  onPick?: (id: string) => void;
  positiveBars?: boolean; signedShares?: boolean;
  unattributed?: DrillUnattributed;
  totalRow?: { label: string; value: string };
  live?: LiveDrillParts;
}) {
  const [adopted, setAdopted] = useState<{ live: LiveDrillParts; parts: readonly DrillPart[]; now: ReturnType<LiveDrillParts['read']> } | null>(null);
  const taken = live && adopted?.live === live && adopted.parts === parts ? adopted.now : null;
  const shownParts = taken?.parts ?? parts, shownUnattributed = taken ? taken.unattributed : unattributed;
  const rows = useMemo(() => drillRows(shownParts, shownUnattributed), [shownParts, shownUnattributed]);
  const rowKeys = rows.map(row => row.key).join('\u0000');
  const given = useRef({ parts, rowKeys });
  given.current = { parts, rowKeys };
  const magnitude = (value: number | null) => positiveBars ? Math.max(0, value ?? 0) : Math.abs(value ?? 0);
  const scaleOf = (shown: readonly { item: DrillPart }[], given: number | null | undefined) => ({
    widest: shown.reduce((result, row) => Math.max(result, magnitude(row.item.value)), 1e-12),
    sum: given === undefined ? drillPartsTotal(shown.map(row => row.item)) : given });
  const { widest, sum } = scaleOf(rows, total);
  const look = (row: DrillPart, scale: { widest: number; sum: number | null }) => ({ amount: amount(row),
    share: showShares ? row.shareText ?? drillPartShare(row.value, scale.sum, signedShares) : '',
    width: `${magnitude(row.value) / scale.widest * 100}%`, negative: row.value != null && row.value < 0,
    background: row.color ?? (row.value != null && row.value < 0 ? 'var(--error)' : 'var(--positive)') });
  const cells = useRef(new Map<string, { amount?: HTMLElement | null; share?: HTMLElement | null; bar?: HTMLElement | null; pick?: HTMLElement | null }>());
  const cell = (key: string, part: 'amount' | 'share' | 'bar' | 'pick') => (node: HTMLElement | null) => {
    const held = cells.current.get(key) ?? {};
    held[part] = node;
    cells.current.set(key, held);
  };
  const totalCell = useRef<HTMLElement | null>(null);
  const looks = useRef({ look, scaleOf });
  looks.current = { look, scaleOf };
  useLayoutEffect(() => {
    if (!live) return;
    let read: ReturnType<LiveDrillParts['read']> | null = null;
    const paint = () => {
      const now = live.read();
      if (now === read) return;
      read = now;
      const shown = drillRows(now.parts, now.unattributed), scale = looks.current.scaleOf(shown, now.total);
      if (shown.map(row => row.key).join('\u0000') !== given.current.rowKeys) {
        setAdopted({ live, parts: given.current.parts, now });
      }
      for (const { key, item } of shown) {
        const held = cells.current.get(key);
        if (!held) continue;
        const next = looks.current.look(item, scale);
        if (held.amount && held.amount.textContent !== next.amount) held.amount.textContent = next.amount;
        held.amount?.classList.toggle('is-negative', next.negative);
        if (held.share && held.share.textContent !== next.share) held.share.textContent = next.share;
        if (held.bar) { held.bar.style.width = next.width; held.bar.style.background = next.background; }
        held.pick?.setAttribute('aria-label', `${item.label}, ${next.amount}`);
      }
      if (totalCell.current && now.totalText !== undefined && totalCell.current.textContent !== now.totalText) totalCell.current.textContent = now.totalText;
    };
    paint();
    const stop = live.subscribe(paint);
    return stop;
  }, [live, rows]);
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
  useEffect(() => () => { reported.current = null; visibleCallback.current?.([]); }, []);
  useEffect(() => {
    if (!active) publishVisible([]);
    else if (!virtual) publishVisible(rows.map(row => row.item.id));
  }, [active, virtual, rows, publishVisible]);
  const selectable = (row: DrillPart) => onPick !== undefined && row.id !== '' && row.selectable !== false;
  const amount = (row: DrillPart) => row.text ?? (row.value == null ? 'Not known' : format(row.value));
  const content = (row: DrillPart, key: string) => {
    const shown = look(row, { widest, sum });
    return <>
      <span className="drill-part-label" data-detail={row.detail ?? row.countryFlag?.nameFailure?.message}>{row.countryFlag !== undefined
        ? <CountryFlagUrl className="drill-part-flag" url={row.countryFlag.flag} detail={row.countryFlag.flagDetail} reason={row.countryFlag.flagReason} pending={row.countryFlag.flagPending} />
        : row.icon && <img src={row.icon} alt="" />}{row.color && <i className="drill-part-swatch" aria-hidden="true" style={{ background: row.color }} />}
        {selectable(row) ? <button ref={cell(key, 'pick')} type="button" tabIndex={0} className="drill-part-pick" aria-label={`${row.label}, ${shown.amount}`}
          onClick={() => onPick?.(row.id)}>{row.label}</button> : row.label}</span>
      <span className="drill-part-bar" aria-hidden="true"><i ref={cell(key, 'bar')} style={{ width: shown.width, background: shown.background }} /></span>
      <b ref={cell(key, 'amount')} className={shown.negative ? 'is-negative' : undefined}>{shown.amount}</b>
      <small ref={cell(key, 'share')}>{showShares ? shown.share : null}</small>
    </>;
  };
  const closing = totalRow && <div className="drill-parts drill-total"><span className="drill-part-label" >{totalRow.label}</span><span /><b ref={totalCell}>{totalRow.value}</b><small /></div>;
  if (!rows.length && !totalRow) return <p className="drill-empty">Nothing recorded for this day.</p>;
  if (virtual) return <div className="drill-inventory">
    <WindowedRows items={active ? rows : []} rowHeight={32} label="Recorded parts" onVisibleRows={visible}>{({ item, key }) => <div tabIndex={selectable(item) ? undefined : 0}
      className={`drill-parts drill-part-row${key === 'unattributed' ? ' is-unattributed' : item.muted || item.value == null ? ' is-muted' : ''}`}>{content(item, key)}</div>}</WindowedRows>
    {closing}
  </div>;
  return <ol className="drill-parts">{rows.map(({ key, item }) => <li key={key} tabIndex={selectable(item) ? undefined : 0}
    className={key === 'unattributed' ? 'is-unattributed' : item.muted || item.value == null ? 'is-muted' : undefined}>{content(item, key)}</li>)}
    {totalRow && <li className="is-total"><span className="drill-part-label" >{totalRow.label}</span><span /><b ref={totalCell}>{totalRow.value}</b><small /></li>}
  </ol>;
}
