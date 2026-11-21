import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import PlaybackSpeed from './PlaybackSpeed';
import '../styles.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle(frames = 6) { for (let at = 0; at < frames; at++) await frame(); }

const speeds: number[] = [];
function Host() {
  const [value, setValue] = useState(1);
  return <div style={{ display: 'flex', justifyContent: 'flex-end', padding: 8 }}>
    <PlaybackSpeed value={value} onChange={speed => { speeds.push(speed); setValue(speed); }} />
  </div>;
}
createRoot(document.body.appendChild(document.createElement('div'))).render(<Host />);

const trigger = () => document.querySelector<HTMLButtonElement>('.header-speed')!;
const panel = () => document.querySelector<HTMLElement>('.playback-speed-menu')!;
const presets = () => [...panel().querySelectorAll<HTMLButtonElement>('.playback-speed-presets button')];

export const fixture = {
  async ready() { await settle(); return trigger() !== null; },
  async openAndClose() {
    trigger().click();
    await settle();
    const opened = panel().matches(':popover-open') && presets().length > 0;
    panel().hidePopover();
    await settle();
    return opened;
  },
  reopened() {
    trigger().click();
    return { open: panel().matches(':popover-open'), visibility: getComputedStyle(panel()).visibility,
      hits: presets().map(button => {
        const box = button.getBoundingClientRect();
        return document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2) === button;
      }) };
  },
  async pick(index: number) {
    await settle();
    presets()[index].click();
    await settle();
    return [...speeds];
  },
};
(window as unknown as { playbackSpeed: typeof fixture }).playbackSpeed = fixture;
