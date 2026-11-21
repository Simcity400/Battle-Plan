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
const chosen: number[] = [], committed: number[] = [];
let cancelled = 0;
createRoot(document.body.appendChild(document.createElement('div'))).render(
  <DateTimePicker label="Date" value={HOUR_EPOCH + 4} min={HOUR_EPOCH} max={HOUR_EPOCH + 24 * 365} otherValue={null} live={live} onChange={at => chosen.push(at)}
    onCommit={at => committed.push(at)} onCancel={() => { cancelled++; }} commitLabel="Go to date" />);

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
  async pickShownTime() {
    const trigger = document.querySelector<HTMLButtonElement>('.calendar-time .select-trigger')!;
    trigger.click();
    await settle();
    await fixture.play(1);
    const before = chosen.length;
    const checked = document.querySelector<HTMLButtonElement>('.select-menu [role="option"][aria-checked="true"]')!;
    const shown = { button: trigger.textContent, checked: checked.textContent };
    checked.click();
    await settle();
    return { ...shown, drafted: chosen.length > before ? chosen.at(-1)! : null, live: hour };
  },
  async commit() {
    document.querySelector<HTMLButtonElement>('.calendar-footer .button.primary')!.click();
    await settle();
    return { committed: [...committed], cancelled, open: !!document.querySelector('.date-time-popover:popover-open') };
  },
  async pickDayThenDismiss(day: number) {
    document.querySelector<HTMLButtonElement>(`.calendar-day[data-day="${day}"]`)!.click();
    await settle();
    document.querySelector<HTMLElement>('.date-time-popover')!.hidePopover();
    await settle();
    return { committed: [...committed], cancelled };
  },
  state() {
    const input = document.querySelector<HTMLInputElement>('.calendar-year')!;
    const hint = document.querySelector('.calendar-year-hint');
    return { value: input.value, invalid: input.getAttribute('aria-invalid'), hint: hint?.textContent ?? null, hintRole: hint?.getAttribute('role') ?? null,
      month: document.querySelector('.calendar-grid')?.getAttribute('aria-label') ?? null,
      go: !document.querySelector<HTMLButtonElement>('.calendar-footer .button.primary')!.disabled, committed: [...committed], cancelled,
      open: !!document.querySelector('.date-time-popover:popover-open') };
  },
  async enterYear(year: string) {
    const input = document.querySelector<HTMLInputElement>('.calendar-year')!;
    input.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, year);
    input.dispatchEvent(new Event('input', { bubbles: true }));
    await settle();
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle();
    return fixture.state();
  },
  async pickDay(day: number) {
    document.querySelector<HTMLButtonElement>(`.calendar-day[data-day="${day}"]`)!.click();
    await settle();
    return fixture.state();
  },
  async browse(direction: 'Previous' | 'Next') {
    document.querySelector<HTMLButtonElement>(`.calendar-arrow[aria-label="${direction} month"]`)!.click();
    await settle();
    return fixture.state();
  },
  async dismiss() { document.querySelector<HTMLElement>('.date-time-popover')!.hidePopover(); await settle(); },
  async enterOnDay(day: number) {
    document.querySelector<HTMLButtonElement>(`.calendar-day[data-day="${day}"]`)!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    await settle();
    return { committed: [...committed], chosen: chosen.at(-1) ?? null, open: !!document.querySelector('.date-time-popover:popover-open') };
  },
};
(window as unknown as { datePicker: typeof fixture }).datePicker = fixture;
