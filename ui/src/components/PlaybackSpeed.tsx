import { Popover, PopoverBody, usePopover } from './Popover';
import { useId, useState } from 'react';
import { Icon } from './Icons';
import { MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED, PLAYBACK_SPEEDS, speedLabel, speedParts, validPlaybackSpeed } from '../dashboard/playback';
import './PlaybackSpeed.css';

const perSecond = (hours: number) => `${hours} game ${hours === 1 ? 'hour' : 'hours'} per second`;
const isPreset = (speed: number) => (PLAYBACK_SPEEDS as readonly number[]).includes(speed);

export default function PlaybackSpeed({ value, onChange }: { value: number; onChange: (speed: number) => void }) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const [custom, setCustom] = useState(String(value));
  const [customOpen, setCustomOpen] = useState(false);
  const speed = Number(custom);
  const valid = custom.trim() !== '' && validPlaybackSpeed(speed);
  const showCustom = customOpen || !isPreset(value);

  const apply = (next: number) => {
    onChange(next);
    menu.close();
  };
  return <>
    <button ref={trigger} type="button" className="header-speed" popoverTarget={id}
      aria-label={`Playback speed: ${speedLabel(value)}`} data-tooltip={perSecond(value)}
      aria-haspopup="dialog" aria-expanded={open}>
      <span>{speedParts(value)[0]}<small> {speedParts(value)[1]}</small></span><Icon name="chevron-down" />
    </button>
    <Popover menu={menu} id={id} className="playback-speed-menu" aria-labelledby={`${id}-heading`}
      onToggle={(event) => { if (event.newState === 'open') { setCustom(String(value)); setCustomOpen(false); } }}>
      <PopoverBody open={open}>{() => <>
        <div className="group-label playback-speed-heading" id={`${id}-heading`}>Playback speed</div>
        <div className="playback-speed-presets">
          {PLAYBACK_SPEEDS.map((preset) => <button key={preset} type="button" data-preset aria-pressed={value === preset}
            data-tooltip={perSecond(preset)} onClick={() => apply(preset)}>{speedLabel(preset)}</button>)}
          <button type="button" className="playback-speed-custom" aria-pressed={!isPreset(value)} aria-expanded={showCustom} aria-controls={`${id}-form`}
            onClick={() => { setCustomOpen(true); requestAnimationFrame(() => document.getElementById(`${id}-custom`)?.focus()); }}>Custom…</button>
        </div>
        {showCustom && <form id={`${id}-form`} onSubmit={(event) => { event.preventDefault(); if (valid) apply(speed); }}>
          <label htmlFor={`${id}-custom`} className="sr-only">Custom speed in game hours per second</label>
          <div><input id={`${id}-custom`} type="number" min={MIN_PLAYBACK_SPEED} max={MAX_PLAYBACK_SPEED} step="any"
            value={custom} onChange={(event) => setCustom(event.currentTarget.value)} required aria-describedby={`${id}-range`} />
            <span className="playback-speed-unit" aria-hidden="true">h/s</span>
            <button type="submit" className="button small" disabled={!valid}>Apply</button></div>
          <small id={`${id}-range`}>Between {MIN_PLAYBACK_SPEED} and {MAX_PLAYBACK_SPEED} game hours per second</small>
        </form>}
      </>}</PopoverBody>
    </Popover>
  </>;
}
