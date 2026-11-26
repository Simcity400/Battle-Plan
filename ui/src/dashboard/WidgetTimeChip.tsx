import { useCallback, useEffect, useId, useSyncExternalStore } from 'react';
import { Icon } from '../components/Icons';
import { Popover, usePopover } from '../components/Popover';
import { formatDateRange, formatDateTime, formatDay } from './calendar.ts';
import type { WidgetRuntime } from './definitions';
import { resolveWindow } from './model.ts';
import type { BoundTrack, WidgetSpec, WidgetTime } from './model';
import { TimeSection } from './settings/TimeSection';

export interface WidgetTimeChipProps {
  widget: WidgetSpec;
  effective: WidgetSpec;
  reference: BoundTrack | undefined;
  runtime: WidgetRuntime;
  modes: readonly WidgetTime['mode'][];
  compact?: boolean;
  onChange: (time: WidgetTime) => void;
  onRegisterOpen?: (open: (() => void) | undefined) => void;
}

export interface TimeChipSummary {
  mode: WidgetTime['mode'];
  label: string;
  tooltip: string;
  chars: number;
}

export function timeChipSummary(widget: Pick<WidgetSpec, 'kind' | 'trackIds' | 'time'>, effective: WidgetSpec,
  reference: BoundTrack | undefined, tracks: readonly BoundTrack[], clockHour = 0): TimeChipSummary {
  const time = widget.time;
  if (time.mode === 'follow') {
    const window = reference ? resolveWindow(effective, reference, tracks as BoundTrack[], clockHour) : null;
    const tooltip = window ? `Live · ${formatDateTime(window.lastHour)}` : 'Live · follows playback';
    return { mode: 'follow', label: 'Live', tooltip, chars: 4 };
  }
  if (time.mode === 'campaign') {
    const tooltip = reference ? `Whole campaign · ${formatDateRange(reference.branch.firstHour, reference.branch.lastHour)}` : 'Whole campaign';
    return { mode: 'campaign', label: 'Whole campaign', tooltip, chars: 14 };
  }
  if (time.mode === 'pin') {
    const pinned = reference ? time.windows[reference.id] : undefined;
    if (pinned && reference) {
      const hour = reference.branch.firstHour + pinned.lastOffset;
      const label = `Pinned · ${formatDay(hour)}`;
      return { mode: 'pin', label, tooltip: `Pinned · ${formatDateTime(hour)}`, chars: label.length };
    }
    return { mode: 'pin', label: 'Pinned', tooltip: 'Pinned moment is not in this campaign', chars: 6 };
  }
  if (reference) {
    const from = reference.branch.firstHour + time.firstOffset;
    const to = reference.branch.firstHour + time.lastOffset;
    const label = formatDateRange(from, to);
    return { mode: 'range', label, tooltip: `${formatDateTime(from)} – ${formatDateTime(to)}`, chars: label.length };
  }
  return { mode: 'range', label: 'Date range', tooltip: 'Date range', chars: 10 };
}

export default function WidgetTimeChip({ widget, effective, reference, runtime, modes, compact = false, onChange, onRegisterOpen }: WidgetTimeChipProps) {
  const id = useId();
  const menu = usePopover('end');
  const readPlaying = useCallback(() => runtime.clock.getSnapshot?.().playing ?? false, [runtime.clock]);
  const subscribeClock = runtime.clock.subscribe ?? (() => () => {});
  const playing = useSyncExternalStore(subscribeClock, readPlaying, readPlaying);
  const interactive = modes.length > 1;
  const summary = timeChipSummary(widget, effective, reference, runtime.tracks, runtime.clock.getHour?.() ?? 0);

  useEffect(() => {
    if (!onRegisterOpen) return;
    if (!interactive) { onRegisterOpen(undefined); return; }
    onRegisterOpen(() => { menu.popup.current?.showPopover(); });
    return () => onRegisterOpen(undefined);
  }, [onRegisterOpen, interactive, menu.popup]);

  const refreshTooltip = useCallback((element: HTMLElement | null) => {
    if (!element || widget.time.mode !== 'follow') return;
    const latest = timeChipSummary(widget, effective, reference, runtime.tracks, runtime.clock.getHour?.() ?? 0);
    element.setAttribute('data-tooltip', latest.tooltip);
  }, [widget, effective, reference, runtime.tracks, runtime.clock]);

  const indicator = summary.mode === 'follow'
    ? <span className={`dash-time-dot${playing ? ' is-playing' : ''}`} aria-hidden="true" />
    : summary.mode === 'pin'
      ? <span className="dash-time-dot is-pinned" aria-hidden="true" />
      : <Icon name="clock" className="dash-time-icon" />;

  if (!interactive) {
    return <span className={`dash-time-chip is-static${compact ? ' is-compact' : ''}`} data-time-mode={summary.mode}
      data-tooltip={summary.tooltip} onPointerEnter={event => refreshTooltip(event.currentTarget)}>
      {indicator}{compact ? <span className="sr-only">{summary.label}</span> : <span className="dash-time-label">{summary.label}</span>}
    </span>;
  }

  return <>
    <button ref={menu.trigger} type="button"
      className={`dash-time-chip${compact ? ' is-compact' : ''}${summary.mode === 'pin' ? ' is-pinned' : ''}${summary.mode === 'follow' ? ' is-live' : ''}`}
      data-time-mode={summary.mode} popoverTarget={id} aria-haspopup="dialog" aria-expanded={menu.open}
      aria-label={`${widget.title}: time, ${summary.label}`} data-tooltip={summary.tooltip}
      onPointerEnter={event => refreshTooltip(event.currentTarget)} onFocus={event => refreshTooltip(event.currentTarget)}>
      {indicator}
      {!compact && <span className="dash-time-label">{summary.label}</span>}
      <Icon name="chevron-down" className="dash-time-chevron" />
    </button>
    <Popover menu={menu} id={id} className="widget-time-menu" aria-label={`${widget.title} time`}>
      {menu.open && <div className="widget-time-menu-body">
        <TimeSection widget={widget} effective={effective} reference={reference} runtime={runtime} modes={modes}
          onChange={time => {
            onChange(time);
            if (time.mode === 'follow' || time.mode === 'campaign') menu.close();
          }} />
      </div>}
    </Popover>
  </>;
}
