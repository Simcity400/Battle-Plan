import { useEffect, useId, useRef, useState } from 'react';
import type { CSSProperties, KeyboardEvent, PointerEvent } from 'react';
import { Popover, usePopover } from '../../components/Popover';
import { Icon } from '../../components/Icons';
import { hsvToRgb, normalizeColor, parseColor, rgbToHsv, toHex } from './color';
import type { Hsv } from './color';

declare global { interface Window { EyeDropper?: new () => { open(): Promise<{ sRGBHex: string }> } } }

const clamp = (value: number) => Math.min(1, Math.max(0, value));

export function ColorPicker({ value, onChange, swatches }: { value: string; onChange: (value: string) => void; swatches: readonly string[] }) {
  const [hsv, setHsv] = useState<Hsv>(() => rgbToHsv(parseColor(value) ?? { r: 0, g: 0, b: 0 }));
  const last = useRef(value);
  useEffect(() => {
    if (value === last.current) return;
    last.current = value;
    const next = rgbToHsv(parseColor(value) ?? { r: 0, g: 0, b: 0 });
    setHsv(previous => next.s === 0 || next.v === 0 ? { ...next, h: previous.h } : next);
  }, [value]);
  const emit = (next: Hsv) => { setHsv(next); const hex = toHex(hsvToRgb(next)); last.current = hex; onChange(hex); };
  const area = useRef<HTMLDivElement>(null);
  const pick = (event: PointerEvent<HTMLDivElement>) => {
    const rect = area.current!.getBoundingClientRect();
    emit({ ...hsv, s: clamp((event.clientX - rect.left) / rect.width), v: clamp(1 - (event.clientY - rect.top) / rect.height) });
  };
  const nudge = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 0.1 : 0.02;
    const moves: Record<string, Partial<Hsv>> = { ArrowLeft: { s: clamp(hsv.s - step) }, ArrowRight: { s: clamp(hsv.s + step) }, ArrowUp: { v: clamp(hsv.v + step) }, ArrowDown: { v: clamp(hsv.v - step) } };
    if (!moves[event.key]) return;
    event.preventDefault();
    emit({ ...hsv, ...moves[event.key] });
  };
  return <div className="color-picker-body" style={{ '--picker-hue': hsv.h } as CSSProperties}>
    <div ref={area} className="color-picker-area" role="slider" tabIndex={0} aria-label="Saturation and brightness"
      aria-valuetext={`Saturation ${Math.round(hsv.s * 100)}%, brightness ${Math.round(hsv.v * 100)}%`}
      onPointerDown={event => { event.currentTarget.setPointerCapture(event.pointerId); pick(event); }}
      onPointerMove={event => { if (event.currentTarget.hasPointerCapture(event.pointerId)) pick(event); }} onKeyDown={nudge}>
      <span className="color-picker-handle" style={{ left: `${hsv.s * 100}%`, top: `${(1 - hsv.v) * 100}%`, background: value }} />
    </div>
    <input className="color-picker-hue" type="range" min={0} max={359} value={Math.round(hsv.h)} aria-label="Hue"
      onChange={event => emit({ ...hsv, h: Number(event.currentTarget.value) })} />
    {swatches.length > 0 && <div className="color-picker-swatches">{swatches.map(swatch => <button key={swatch} type="button" style={{ background: swatch }}
      aria-label={`Use ${swatch}`} aria-pressed={swatch === value} onClick={() => { last.current = swatch; setHsv(rgbToHsv(parseColor(swatch)!)); onChange(swatch); }} />)}</div>}
  </div>;
}

export default function ColorField({ label, value, onChange, swatches = [], help }: { label: string; value: string; onChange: (value: string) => void; swatches?: readonly string[]; help?: string }) {
  const id = useId();
  const menu = usePopover('start');
  const [text, setText] = useState(value);
  useEffect(() => setText(value), [value]);
  const commit = () => { const next = normalizeColor(text); if (next) onChange(next); else setText(value); };
  const eyedropper = typeof window !== 'undefined' && window.EyeDropper;
  return <div className="color-field">
    <button ref={menu.trigger} type="button" className="color-swatch" style={{ background: value }} popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}
      aria-label={`Choose ${label.toLowerCase()} colour`} />
    <span className="color-field-label">{label}{help && <span className="folder-info" tabIndex={0} role="img" aria-label={`About ${label}`} data-tooltip={help}><Icon name="info" /></span>}</span>
    <input className="color-hex" aria-label={`${label} hex value`} value={text} spellCheck={false} maxLength={9}
      onChange={event => setText(event.currentTarget.value)} onBlur={commit} onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commit(); } }} />
    <Popover menu={menu} id={id} className="color-picker" aria-label={`${label} colour`}>
      <div className="ui-popover-heading"><strong>{label}</strong><div className="ui-popover-actions">
        {eyedropper && <button type="button" className="icon-button ui-popover-action" aria-label="Pick a colour from the screen" data-tooltip="Pick from screen"
          onClick={() => { void new window.EyeDropper!().open().then(result => { const next = normalizeColor(result.sRGBHex); if (next) onChange(next); }).catch(() => {}); }}><Icon name="sparkle" /></button>}
        <button type="button" className="icon-button ui-popover-action" aria-label="Close colour picker" onClick={menu.close}><Icon name="close" /></button></div></div>
      {menu.open && <ColorPicker value={value} onChange={onChange} swatches={swatches} />}
    </Popover>
  </div>;
}
