import { useId, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent } from 'react';
import { Icon } from '../Icons';
import {
  engineHour, firstHourOfMonth, formatDate, formatDateTime, formatTime, gameDate, HOUR_EPOCH,
  monthAt, monthIndex, monthLengths, monthNames,
} from './calendar';
import './DateTimePicker.css';

interface Props {
  label: string;
  value: number | null;
  min: number;
  max: number;
  otherValue: number | null;
  disabled?: boolean;
  invalid?: boolean;
  onChange: (hour: number) => void;
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

export default function DateTimePicker({ label, value, min, max, otherValue, disabled, invalid, onChange }: Props) {
  const id = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const focusDay = useRef(false);
  const selected = value == null ? null : gameDate(value);
  const minimum = gameDate(Math.max(min, HOUR_EPOCH));
  const maximum = gameDate(max);
  const available = !disabled && minimum != null && maximum != null && min <= max;
  const initial = selected ?? minimum;
  const [open, setOpen] = useState(false);
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
  const selectedDayStart = value != null && selected ? value - selected.hour + 1 : min;
  const rangeFirst = value != null && otherValue != null ? Math.min(value, otherValue) : null;
  const rangeLast = value != null && otherValue != null ? Math.max(value, otherValue) : null;

  useLayoutEffect(() => {
    if (!open) return;
    const position = () => {
      if (!trigger.current || !popup.current) return;
      const anchor = trigger.current.getBoundingClientRect();
      const { width, height } = popup.current.getBoundingClientRect();
      const viewport = document.documentElement;
      const below = anchor.bottom + 8;
      const top = below + height <= viewport.clientHeight - 12 ? below : anchor.top - height - 8;
      popup.current.style.left = `${clamp(anchor.left, 12, viewport.clientWidth - width - 12)}px`;
      popup.current.style.top = `${clamp(top, 12, viewport.clientHeight - height - 12)}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    if (popup.current) observer.observe(popup.current);
    if (trigger.current) observer.observe(trigger.current);
    window.addEventListener('resize', position);
    window.addEventListener('scroll', position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', position);
      window.removeEventListener('scroll', position, true);
    };
  }, [open]);

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
    onChange(hour);
    setFocusedDay(next.day);
    showMonth(monthIndex(next));
  }

  function chooseDay(day: number) {
    // Preserve time where possible; a partially recorded boundary day can
    // restrict the available hours. The time field always shows the result.
    const start = engineHour({ ...view, day, hour: 1 });
    if (start != null) choose(clamp(start + (selected?.hour ?? 1) - 1, min, max));
  }

  function commitYear(focus = false) {
    const year = Number(yearInput);
    if (/^-?\d+$/.test(yearInput) && Number.isSafeInteger(year) && minimum && maximum) {
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
        showMonth(viewMonth + (event.key === 'PageUp' ? -1 : 1) * (event.shiftKey ? 12 : 1), true);
        return;
      }
      default: return;
    }
    event.preventDefault();
    const next = gameDate(clamp(monthStart + (day - 1 + step) * 24, min, max));
    if (!next) return;
    setFocusedDay(next.day);
    showMonth(monthIndex(next), true);
  }

  return <div className="field date-time-field">
    <span id={`${id}-label`}>{label}</span>
    <button type="button" ref={trigger} className="date-time-trigger" disabled={!available}
      aria-labelledby={`${id}-label ${id}-value`} aria-describedby="date-range-help range-error"
      aria-haspopup="dialog" aria-expanded={open} aria-invalid={invalid}
      title={!available && value != null ? 'This part of the recording has no calendar date.' : undefined}
      popoverTarget={`${id}-popup`}>
      <Icon name="calendar" />
      <span id={`${id}-value`} className="date-time-value">
        <span>{selected ? formatDate(selected) : value != null ? formatDateTime(value) : 'Choose date'}</span>
        <span className="date-time-hour">{selected ? formatTime(selected.hour) : '—'}</span>
      </span>
      <Icon name="chevron-down" />
    </button>
    <div ref={popup} id={`${id}-popup`} popover="auto" className="date-time-popover"
      role="dialog" aria-label={`${label} date and time`} onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          event.stopPropagation();
          popup.current?.hidePopover();
          trigger.current?.focus();
        }
      }} onToggle={(event) => {
        const isOpen = event.newState === 'open';
        if (isOpen && initial) {
          showMonth(monthIndex(initial), true);
          setFocusedDay(initial.day);
        }
        setOpen(isOpen);
      }}>
      <div className="calendar-navigation">
        <button type="button" className="calendar-arrow" aria-label="Previous month" disabled={viewMonth <= firstMonth}
          onClick={() => showMonth(viewMonth - 1)}><Icon name="chevron-left" /></button>
        <label className="sr-only" htmlFor={`${id}-month`}>Month</label>
        <select id={`${id}-month`} value={view.month} onChange={(event) => showMonth(view.year * 12 + Number(event.currentTarget.value) - 1)}>
          {monthNames.map((name, index) => <option key={name} value={index + 1}
            disabled={view.year * 12 + index < firstMonth || view.year * 12 + index > lastMonth}>{name}</option>)}
        </select>
        <label className="sr-only" htmlFor={`${id}-year`}>Year</label>
        <input id={`${id}-year`} className="calendar-year" type="number" step="1" min={minimum?.year} max={maximum?.year}
          value={yearInput} onChange={(event) => setYearInput(event.currentTarget.value)} onBlur={() => commitYear()}
          onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); commitYear(true); } }} />
        <button type="button" className="calendar-arrow" aria-label="Next month" disabled={viewMonth >= lastMonth}
          onClick={() => showMonth(viewMonth + 1)}><Icon name="chevron-right" /></button>
      </div>
      <p className="sr-only" aria-live="polite">{monthNames[view.month - 1]} {view.year}</p>
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
            return <div role="gridcell" aria-selected={isSelected} key={column}>
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
        <select id={`${id}-time`} value={selected?.hour ?? 1} disabled={!selected}
          onChange={(event) => choose(selectedDayStart + Number(event.currentTarget.value) - 1)}>
          {Array.from({ length: 24 }, (_, index) => <option key={index} value={index + 1}
            disabled={selectedDayStart + index < min || selectedDayStart + index > max}>{formatTime(index + 1)}</option>)}
        </select>
      </div>
      <div className="calendar-footer">
        <div className="calendar-shortcuts">
          <button type="button" className="text-button" onClick={() => choose(Math.max(min, HOUR_EPOCH))}>{min < HOUR_EPOCH ? 'First calendar date' : 'Branch start'}</button>
          <button type="button" className="text-button" onClick={() => choose(max)}>Branch end</button>
        </div>
        <button type="button" className="button primary small" onClick={() => { popup.current?.hidePopover(); trigger.current?.focus(); }}>Done</button>
      </div>
      <span className="sr-only" aria-live="polite">{value != null ? formatDateTime(value) : 'Choose a date'}</span>
    </div>
  </div>;
}
