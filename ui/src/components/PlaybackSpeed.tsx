import { Popover, usePopover } from './Popover';
import { useId, useState } from 'react';
import { Icon } from './Icons';
import { MAX_PLAYBACK_SPEED, MIN_PLAYBACK_SPEED, PLAYBACK_SPEEDS, validPlaybackSpeed } from '../dashboard/playback';
import './PlaybackSpeed.css';

export default function PlaybackSpeed({ value, onChange }: { value: number; onChange: (speed: number) => void }) {
  const id = useId();
  const menu = usePopover();
  const { trigger, open } = menu;
  const [custom, setCustom] = useState(String(value));
  const speed = Number(custom);
  const valid = custom.trim() !== '' && validPlaybackSpeed(speed);

  const apply = (next: number) => {
    onChange(next);
    menu.close();
  };
  return <>
    <button ref={trigger} type="button" className="header-speed" popoverTarget={id}
      aria-label={`Playback speed: ${value} hours per second`} data-tooltip="Game hours per second"
      aria-haspopup="dialog" aria-expanded={open}>
      <span>{value}<small> h/s</small></span><Icon name="chevron-down" />
    </button>
    <Popover menu={menu} id={id} className="playback-speed-menu" aria-label="Playback speed"
      onToggle={(event) => { const next = event.newState === 'open'; if (next) setCustom(String(value)); }}>
      <div className="popover-label">Game hours per second</div>
      <div className="playback-speed-presets">
        {PLAYBACK_SPEEDS.map((preset) => <button key={preset} type="button" aria-pressed={value === preset} onClick={() => apply(preset)}>{preset}</button>)}
      </div>
      <form onSubmit={(event) => { event.preventDefault(); if (valid) apply(speed); }}>
        <label htmlFor={`${id}-custom`} className="popover-label">Custom speed</label>
        <div><input id={`${id}-custom`} type="number" min={MIN_PLAYBACK_SPEED} max={MAX_PLAYBACK_SPEED} step="any"
          value={custom} onChange={(event) => setCustom(event.currentTarget.value)} required aria-describedby={`${id}-range`} />
          <button type="submit" className="button small" disabled={!valid}>Apply</button></div>
        <small id={`${id}-range`}>Between {MIN_PLAYBACK_SPEED} and {MAX_PLAYBACK_SPEED} h/s</small>
      </form>
    </Popover>
  </>;
}
