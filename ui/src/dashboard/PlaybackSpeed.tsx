import { useId, useLayoutEffect, useRef, useState } from 'react';
import { Icon } from '../Icons';
import { MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED, PLAYBACK_SPEEDS, validPlaybackSpeed } from './playback';

export default function PlaybackSpeed({ value, onChange }: { value: number; onChange: (speed: number) => void }) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [custom, setCustom] = useState(String(value));
  const speed = Number(custom);
  const valid = custom.trim() !== '' && validPlaybackSpeed(speed);
  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      if (!trigger.current || !popup.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const { width, height } = popup.current.getBoundingClientRect();
      popup.current.style.left = `${Math.max(12, Math.min(anchor.right - width, document.documentElement.clientWidth - width - 12))}px`;
      popup.current.style.top = `${Math.max(12, Math.min(anchor.bottom + 16, document.documentElement.clientHeight - height - 12))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    if (trigger.current) observer.observe(trigger.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => { observer.disconnect(); window.removeEventListener('resize', position); window.removeEventListener('scroll', position, true); };
  }, [open]);
  const apply = (next: number) => {
    onChange(next);
    popup.current?.hidePopover();
    trigger.current?.focus();
  };
  return <>
    <button ref={trigger} type="button" className="header-speed" popoverTarget={id}
      aria-label={`Playback speed: ${value} hours per second`} data-tooltip={`${value} game hours per second`}
      aria-haspopup="dialog" aria-expanded={open}>
      <span>{value} h/s</span><Icon name="chevron-down" />
    </button>
    <div ref={popup} id={id} popover="auto" className="playback-speed-menu" role="dialog" aria-label="Playback speed"
      onToggle={(event) => { const next = event.newState === 'open'; setOpen(next); if (next) setCustom(String(value)); }}
      onKeyDown={(event) => { if (event.key === 'Escape') { popup.current?.hidePopover(); trigger.current?.focus(); } }}>
      <p>Game hours per second</p>
      <div className="playback-speed-presets">
        {PLAYBACK_SPEEDS.map((preset) => <button key={preset} type="button" aria-pressed={value === preset} onClick={() => apply(preset)}>
          <span>{preset} h/s</span>{value === preset && <span aria-hidden="true">✓</span>}
        </button>)}
      </div>
      <form onSubmit={(event) => { event.preventDefault(); if (valid) apply(speed); }}>
        <label htmlFor={`${id}-custom`}>Custom hours per second</label>
        <div><input id={`${id}-custom`} type="number" min={MIN_PLAYBACK_SPEED} max={MAX_PLAYBACK_SPEED} step="any"
          value={custom} onChange={(event) => setCustom(event.currentTarget.value)} required aria-describedby={`${id}-range`} />
          <button type="submit" className="button small" disabled={!valid}>Apply</button></div>
        <small id={`${id}-range`}>{MIN_PLAYBACK_SPEED}–{MAX_PLAYBACK_SPEED} h/s</small>
      </form>
    </div>
  </>;
}
