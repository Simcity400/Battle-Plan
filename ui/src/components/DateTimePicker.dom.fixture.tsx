import { createRoot } from 'react-dom/client';
import DateTimePicker from './DateTimePicker';
import type { LiveValue } from './DateTimePicker';
import { gameDate, HOUR_EPOCH } from '../dashboard/calendar.ts';
import '../styles.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle(frames = 6) { for (let at = 0; at < frames; at++) await frame(); }

let hour = HOUR_EPOCH + 4;
const listeners = new Set<() => void>();
const live: LiveValue = { read: () => hour, subscribe: changed => { listeners.add(changed); return () => listeners.delete(changed); } };
const chosen: number[] = [];
createRoot(document.body.appendChild(document.createElement('div'))).render(
  <DateTimePicker label="Date" value={HOUR_EPOCH + 4} min={HOUR_EPOCH} max={HOUR_EPOCH + 24 * 365} otherValue={null} live={live} onChange={at => chosen.push(at)} />);

const dayStart = (at: number) => at - gameDate(at)!.hour + 1;

export const fixture = {
  async ready() { await settle(); return document.querySelector('.date-time-trigger') !== null; },
  async open() { document.querySelector<HTMLButtonElement>('.date-time-trigger')!.click(); await settle(); },
  async play(hours: number) { hour += hours; for (const changed of [...listeners]) changed(); await settle(); },
  selectedDay() { return document.querySelector('.calendar-grid [role="gridcell"][aria-selected="true"] [data-day]')?.getAttribute('data-day') ?? null; },
  async pickTime(time: number) {
    document.querySelector<HTMLButtonElement>('.calendar-time .select-trigger')!.click();
    await settle();
    document.querySelector<HTMLButtonElement>(`.select-menu [role="option"][data-option-index="${time - 1}"]`)!.click();
    await settle();
    return { chosen: chosen.at(-1) ?? null, wanted: dayStart(hour) + time - 1 };
  },
};
(window as unknown as { datePicker: typeof fixture }).datePicker = fixture;
