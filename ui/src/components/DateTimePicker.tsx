import { Popover, PopoverBody, usePopover } from './Popover';
import { Select } from './Select';
import { useId, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Icon } from './Icons';
import {
  engineHour, firstHourOfMonth, formatDate, formatDateTime, formatTime, gameDate, HOUR_EPOCH,
  monthAt, monthIndex, monthLengths, monthNames,
} from '../dashboard/calendar.ts';
import './DateTimePicker.css';

interface Props {
  label: string;
  hideLabel?: boolean;
  value: number | null;
  min: number;
  max: number;
  otherValue: number | null;
  disabled?: boolean;
  invalid?: boolean;
  describedBy?: string;
  id?: string;
  onChange: (hour: number) => void;
  /** A value that changes too often to render: the trigger's date and time are painted from it, and an open
   * calendar follows it until the user pages or picks; `value` drives the calendar otherwise. */
  live?: LiveValue;
}

export interface LiveValue { read: () => number | null; subscribe: (changed: () => void) => () => void }

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
const dateText = (date: ReturnType<typeof gameDate>, value: number | null) =>
  date ? formatDate(date) : value != null ? formatDateTime(value) : 'Choose date';

/** Changes the text in place, so a date painted every frame inserts no nodes and leaves style alone. */
function showText(element: HTMLElement, text: string): void {
  const node = element.firstChild;
  if (node instanceof Text && node === element.lastChild) { if (node.data !== text) node.data = text; } else element.textContent = text;
}

export default function DateTimePicker({ label, hideLabel = false, value, min, max, otherValue, disabled, invalid, describedBy = 'date-range-help range-error', id: suppliedId, onChange, live }: Props) {
  const generatedId = useId();
  const id = suppliedId ?? generatedId;
  const menu = usePopover('start');
  const { trigger, popup, open } = menu;
  const focusDay = useRef(false);
  // Open on a live value, the calendar shows the live hour (`held`, as of its last render). While `following`, the
  // day and time are painted in the frame and only a new month renders; paging or keys hold it where it is.
  const [held, setHeld] = useState<{ hour: number; following: boolean } | null>(null);
  const shownValue = live && held ? held.hour : value;
  const selected = shownValue == null ? null : gameDate(shownValue);
  const minimum = gameDate(Math.max(min, HOUR_EPOCH));
  const maximum = gameDate(max);
  const available = !disabled && minimum != null && maximum != null && min <= max;
  const initial = selected ?? minimum;
  const [focusRequest, setFocusRequest] = useState(0);
  const [viewMonth, setViewMonth] = useState(() => initial ? monthIndex(initial) : 0);
  const [focusedDay, setFocusedDay] = useState(initial?.day ?? 1);
  const [yearInput, setYearInput] = useState(String(initial?.year ?? 0));
  const view = monthAt(viewMonth);
  const firstMonth = minimum ? monthIndex(minimum) : viewMonth;
  const lastMonth = maximum ? monthIndex(maximum) : viewMonth;
  const monthStart = firstHourOfMonth(viewMonth);
  const daysInMonth = monthLengths[view.month - 1];
  const firstDay = clamp(Math.floor((min - monthStart) / 24) + 1, 1, daysInMonth);
  const lastDay = clamp(Math.floor((max - monthStart) / 24) + 1, 1, daysInMonth);
  const activeDay = clamp(focusedDay, firstDay, lastDay);
  const selectedDayStart = shownValue != null && selected ? shownValue - selected.hour + 1 : min;
  const rangeFirst = shownValue != null && otherValue != null ? Math.min(shownValue, otherValue) : null;
  const rangeLast = shownValue != null && otherValue != null ? Math.max(shownValue, otherValue) : null;
  const following = open && !!live && !!held?.following;
  const liveDate = useRef<HTMLSpanElement>(null);
  const liveTime = useRef<HTMLSpanElement>(null);

  useLayoutEffect(() => {
    if (!live) return;
    let shown: number | null | undefined;
    const paint = () => {
      const current = live.read();
      if (current === shown || !liveDate.current || !liveTime.current) return;
      shown = current;
      const date = current == null ? null : gameDate(current);
      showText(liveDate.current, dateText(date, current));
      showText(liveTime.current, date ? formatTime(date.hour) : '—');
    };
    paint();
    return live.subscribe(paint);
  }, [live]);

  const paintHead = useRef<(() => void) | null>(null);
  paintHead.current = following ? () => {
    const hour = live!.read();
    const date = hour == null ? null : gameDate(hour);
    const body = popup.current;
    if (hour == null || !date || !body) return;
    if (monthIndex(date) !== viewMonth) {
      setHeld({ hour, following: true });
      setFocusedDay(date.day);
      showMonth(monthIndex(date));
      return;
    }
    for (const cell of body.querySelectorAll<HTMLElement>('.calendar-grid [role="gridcell"]')) {
      const day = cell.firstElementChild?.getAttribute('data-day');
      // A day cell becomes an empty one as the month changes, keeping what was painted on it.
      if (day == null) { cell.removeAttribute('aria-selected'); continue; }
      const on = String(Number(day) === date.day);
      if (cell.getAttribute('aria-selected') !== on) cell.setAttribute('aria-selected', on);
    }
    const time = body.querySelector<HTMLElement>('.calendar-time .select-trigger > span');
    if (time) showText(time, formatTime(date.hour));
  } : null;
  // After every render, since React leaves the followed day and time to the painter, and on every live change.
  useLayoutEffect(() => paintHead.current?.());
  useLayoutEffect(() => following ? live!.subscribe(() => paintHead.current?.()) : undefined, [following, live]);

  /** The user moved the calendar: it stops following and holds the hour it shows. */
  function hold() {
    if (!following) return;
    const hour = live!.read();
    setHeld(hour == null ? null : { hour, following: false });
  }

  useLayoutEffect(() => {
    if (!open || !focusDay.current) return;
    focusDay.current = false;
    popup.current?.querySelector<HTMLButtonElement>(`[data-day="${activeDay}"]`)?.focus({ preventScroll: true });
  }, [open, viewMonth, activeDay, focusRequest]);

  function showMonth(index: number, focus = false) {
    const next = clamp(index, firstMonth, lastMonth);
    setViewMonth(next);
    setYearInput(String(monthAt(next).year));
    focusDay.current = focus;
    if (focus) setFocusRequest((request) => request + 1);
  }

  function choose(hour: number) {
    const next = gameDate(hour);
    if (!next || hour < min || hour > max) return;
    setHeld(null);
    onChange(hour);
    setFocusedDay(next.day);
    showMonth(monthIndex(next));
  }

  /** The hour the calendar shows now: following, the live one its painter shows, which `held` trails. */
  function current(): { hour: number; date: NonNullable<ReturnType<typeof gameDate>> } | null {
    const hour = following ? live!.read() : shownValue;
    const date = hour == null ? null : gameDate(hour);
    return hour != null && date ? { hour, date } : null;
  }

  function chooseDay(day: number) {
    // Preserve time where possible; a partially recorded boundary day can
    // restrict the available hours. The time field always shows the result.
    const start = engineHour({ ...view, day, hour: 1 });
    if (start != null) choose(clamp(start + (current()?.date.hour ?? 1) - 1, min, max));
  }

  function chooseTime(time: number) {
    const shown = current();
    choose((shown ? shown.hour - shown.date.hour + 1 : min) + time - 1);
  }

  function commitYear(focus = false) {
    const year = Number(yearInput);
    if (/^-?\d+$/.test(yearInput) && Number.isSafeInteger(year) && minimum && maximum) {
      if (year !== view.year) hold();
      showMonth(clamp(year, minimum.year, maximum.year) * 12 + view.month - 1, focus);
    } else {
      setYearInput(String(view.year));
    }
  }

  function moveFocus(event: KeyboardEvent<HTMLButtonElement>, day: number) {
    let step: number;
    switch (event.key) {
      case 'ArrowLeft': step = -1; break;
      case 'ArrowRight': step = 1; break;
      case 'ArrowUp': step = -7; break;
      case 'ArrowDown': step = 7; break;
      case 'Home': step = -((day - 1) % 7); break;
      case 'End': step = Math.min(6 - (day - 1) % 7, daysInMonth - day); break;
      case 'PageUp':
      case 'PageDown': {
        event.preventDefault();
        hold();
        showMonth(viewMonth + (event.key === 'PageUp' ? -1 : 1) * (event.shiftKey ? 12 : 1), true);
        return;
      }
      default: return;
    }
    event.preventDefault();
    hold();
    const next = gameDate(clamp(monthStart + (day - 1 + step) * 24, min, max));
    if (!next) return;
    setFocusedDay(next.day);
    showMonth(monthIndex(next), true);
  }

  return <div className="field date-time-field">
    <span id={`${id}-label`} className={hideLabel ? 'sr-only' : undefined}>{label}</span>
    <button type="button" ref={trigger} className="date-time-trigger" disabled={!available}
      aria-labelledby={`${id}-label ${id}-value`} aria-describedby={describedBy}
      aria-haspopup="dialog" aria-expanded={open} aria-invalid={invalid}
      data-tooltip={!available && value != null ? 'This part of the recording has no calendar date.' : undefined}
      popoverTarget={`${id}-popup`}>
      <Icon name="calendar" />
      <span id={`${id}-value`} className="date-time-value">
        {live ? <><span key="live-date" ref={liveDate} /><span key="live-time" ref={liveTime} className="date-time-hour" /></>
          : <><span key="date">{dateText(selected, value)}</span><span key="time" className="date-time-hour">{selected ? formatTime(selected.hour) : '—'}</span></>}
      </span>
      <Icon name="chevron-down" />
    </button>
    <Popover menu={menu} id={`${id}-popup`} className="date-time-popover"
      aria-label={`${label} date and time`} onToggle={(event) => {
        const isOpen = event.newState === 'open';
        // A live value opens on the date shown now, which `value` may trail.
        const shown = live ? live.read() : value;
        const opening = (shown == null ? null : gameDate(shown)) ?? minimum;
        setHeld(isOpen && live && shown != null ? { hour: shown, following: true } : null);
        if (isOpen && opening) {
          showMonth(monthIndex(opening), true);
          setFocusedDay(opening.day);
        }
      }}>
      <PopoverBody open={open}>{() => <>
        <div className="calendar-navigation">
          <button type="button" className="calendar-arrow" aria-label="Previous month" disabled={viewMonth <= firstMonth}
            onClick={() => { hold(); showMonth(viewMonth - 1); }}><Icon name="chevron-left" /></button>
          <Select label="Month" value={String(view.month)} onChange={value => { hold(); showMonth(view.year * 12 + Number(value) - 1); }}
            options={monthNames.map((name, index) => ({ value: String(index + 1), label: name,
              disabled: view.year * 12 + index < firstMonth || view.year * 12 + index > lastMonth }))} />
          <label className="sr-only" htmlFor={`${id}-year`}>Year</label>
          <input id={`${id}-year`} className="calendar-year" type="number" step="1" min={minimum?.year} max={maximum?.year}
            value={yearInput} onChange={(event) => setYearInput(event.currentTarget.value)} onBlur={() => commitYear()}
            onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); commitYear(true); } }} />
          <button type="button" className="calendar-arrow" aria-label="Next month" disabled={viewMonth >= lastMonth}
            onClick={() => { hold(); showMonth(viewMonth + 1); }}><Icon name="chevron-right" /></button>
        </div>
        {/* Following a live value, the month and date move on their own: announced only once the user moves them. */}
        <p className="sr-only" aria-live={following ? 'off' : 'polite'}>{monthNames[view.month - 1]} {view.year}</p>
        <p id={`${id}-keys`} className="sr-only">Use arrow keys to move between days. Page Up and Page Down change the month; hold Shift to change the year.</p>
        {/* This is the game's fixed-length calendar. Weekday names would imply
            a Gregorian leap-year calendar that these recordings do not use. */}
        <div className="calendar-grid" role="grid" aria-label={`${monthNames[view.month - 1]} ${view.year}`}
          aria-describedby={`${id}-keys`}>
          {Array.from({ length: Math.ceil(daysInMonth / 7) }, (_, row) => <div className="calendar-week" role="row" key={row}>
            {Array.from({ length: 7 }, (_, column) => {
              const day = row * 7 + column + 1;
              if (day > daysInMonth) return <div role="gridcell" key={column} />;
              const start = monthStart + (day - 1) * 24;
              const isSelected = selected != null && monthIndex(selected) === viewMonth && selected.day === day;
              const isDisabled = start > max || start + 23 < min;
              const inRange = rangeFirst != null && rangeLast != null && start <= rangeLast && start + 23 >= rangeFirst;
              return <div role="gridcell" aria-selected={following ? undefined : isSelected} key={column}>
                <button type="button" data-day={day} className={`calendar-day${inRange ? ' in-range' : ''}`}
                  aria-label={`${day} ${monthNames[view.month - 1]} ${view.year}`} disabled={isDisabled}
                  tabIndex={day === activeDay ? 0 : -1} onFocus={() => setFocusedDay(day)}
                  onKeyDown={(event) => moveFocus(event, day)} onClick={() => chooseDay(day)}>{day}</button>
              </div>;
            })}
          </div>)}
        </div>
        <div className="calendar-time">
          <label htmlFor={`${id}-time`}><Icon name="clock" /><span>Time<span className="sr-only"> · Game time, hourly</span></span></label>
          <Select id={`${id}-time`} label="Time" value={String(selected?.hour ?? 1)} disabled={!selected}
            onChange={value => chooseTime(Number(value))}
            options={Array.from({ length: 24 }, (_, index) => ({ value: String(index + 1), label: formatTime(index + 1),
              disabled: selectedDayStart + index < min || selectedDayStart + index > max }))} />
        </div>
        <div className="calendar-footer">
          <div className="calendar-shortcuts">
            <button type="button" className="text-button" data-tooltip="Select the first available date" onClick={() => choose(Math.max(min, HOUR_EPOCH))}>First date</button>
            <button type="button" className="text-button" data-tooltip="Select the last available date" onClick={() => choose(max)}>Last date</button>
          </div>
          <button type="button" className="button primary small" onClick={menu.close}>Done</button>
        </div>
        <span className="sr-only" aria-live={following ? 'off' : 'polite'}>{shownValue != null ? formatDateTime(shownValue) : 'Choose a date'}</span>
      </>}</PopoverBody>
    </Popover>
  </div>;
}
