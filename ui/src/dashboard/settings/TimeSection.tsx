import { useState } from 'react';
import { Segmented } from '../../components/Segmented';
import { SettingLabel } from './SettingLabel';
import DateTimePicker from '../../components/DateTimePicker';
import { formatDateTime } from '../calendar';
import { pinNow } from '../regionGate';
import { axisHour } from '../regionTime';
import type { BoundTrack, WidgetSpec, WidgetTime } from '../model';
import type { WidgetRuntime } from '../definitions';

const MODES: Record<WidgetTime['mode'], { label: string; tooltip: string }> = {
  follow: { label: 'Live', tooltip: 'Follow the playback clock' },
  campaign: { label: 'Whole campaign', tooltip: 'Show the whole campaign as it stood at the end' },
  pin: { label: 'Pinned', tooltip: 'Hold one moment while playback moves on' },
  range: { label: 'Date range', tooltip: 'Show a fixed stretch of the campaign' },
};
const ORDER: readonly WidgetTime['mode'][] = ['follow', 'campaign', 'pin', 'range'];

export function TimeSection({ widget, effective, reference, runtime, modes, onChange }: {
  widget: WidgetSpec; effective: WidgetSpec; reference: BoundTrack | undefined; runtime: WidgetRuntime;
  modes: readonly WidgetTime['mode'][]; onChange: (time: WidgetTime) => void;
}) {
  const [refused, setRefused] = useState<string | null>(null);
  const offered = ORDER.filter(mode => modes.includes(mode));
  const needsPin = !reference && offered.includes('pin');
  const needsRange = !reference && offered.includes('range');
  const campaignNote = needsPin && needsRange ? 'Load a campaign to pin a moment or choose a date range.'
    : needsPin ? 'Load a campaign to pin a moment.'
      : needsRange ? 'Load a campaign to choose a date range.' : null;
  const choose = (mode: WidgetTime['mode']) => {
    setRefused(null);
    if (mode === 'follow' || mode === 'campaign') onChange({ mode });
    else if (mode === 'pin') {
      const pin = pinNow(effective, runtime.tracks, axisHour(runtime.clock));
      if (pin.time) onChange(pin.time.value); else setRefused(`Can’t pin now: ${pin.refused}`);
    }
    else if (reference) onChange({ mode: 'range', firstOffset: 0, lastOffset: reference.branch.lastHour - reference.branch.firstHour });
  };
  const time = widget.time;
  const pinned = time.mode === 'pin' && reference ? time.windows[reference.id] : undefined;
  return <>
    {offered.length > 1 && <Segmented label="Time" value={time.mode} onChange={choose}
      options={offered.map(mode => {
        const unavailable = !reference && (mode === 'pin' || mode === 'range');
        return {
          value: mode,
          ...MODES[mode],
          disabled: unavailable,
          reason: unavailable ? (mode === 'pin' ? 'Load a campaign to pin a moment.' : 'Load a campaign to choose a date range.') : undefined,
        };
      })} />}
    {campaignNote && <p className="field-hint">{campaignNote}</p>}
    {refused && <p className="setting-row" role="alert">{refused}</p>}
    {time.mode === 'pin' && <div className="setting-row"><SettingLabel label={pinned && reference
      ? <>Holding {formatDateTime(reference.branch.firstHour + pinned.lastOffset)}</> : 'The pinned moment is not in this campaign'} />
      <button type="button" className="button small" disabled={!reference} onClick={() => choose('pin')}
        data-tooltip={reference ? undefined : 'Load this campaign to pin a new moment'}>Pin to now</button></div>}
    {time.mode === 'range' && reference && <RangeFields time={time} reference={reference} onChange={onChange} />}
  </>;
}

function RangeFields({ time, reference, onChange }: { time: Extract<WidgetTime, { mode: 'range' }>; reference: BoundTrack; onChange: (time: WidgetTime) => void }) {
  const { firstHour, lastHour } = reference.branch;
  const from = firstHour + time.firstOffset;
  const to = firstHour + time.lastOffset;
  const set = (first: number, last: number) => onChange({ mode: 'range', firstOffset: first - firstHour, lastOffset: last - firstHour });
  return <div className="settings-range">
    <DateTimePicker label="From" value={from} min={firstHour} max={lastHour} otherValue={to} describedBy="" onChange={hour => set(hour, Math.max(hour, to))} />
    <DateTimePicker label="To" value={to} min={firstHour} max={lastHour} otherValue={from} describedBy="" onChange={hour => set(Math.min(hour, from), hour)} />
  </div>;
}
