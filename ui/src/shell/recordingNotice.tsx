import type { DamagedRecording, PartialRecording, RecordingGap } from '../generated/dashboard';
import { formatDate, formatDateTime, formatTime, gameDate } from '../dashboard/calendar';
import { recordingDisplayName } from '../dashboard/recordingNames';
import { Icon } from '../components/Icons';

export function recordedHour(hour: number): string {
  const date = gameDate(hour);
  return date ? `${formatDate(date)} ${formatTime(date.hour)}` : formatDateTime(hour);
}

const hours = (first: number, last: number) => first === last ? recordedHour(first) : `${recordedHour(first)} – ${recordedHour(last)}`;

export function gapText(gap: Pick<RecordingGap, 'firstHour' | 'lastHour'>): string {
  return `Not recorded · ${hours(gap.firstHour, gap.lastHour)}`;
}

export function partialRecordingDetail(partial: PartialRecording): string {
  return partial.failedHour === null ? 'part of the recording couldn’t be saved' : `${recordedHour(partial.failedHour)} couldn’t be recorded`;
}

export function partialRecordingText(partial: PartialRecording): string {
  return `Partial recording · ${partialRecordingDetail(partial)}`;
}

export function damagedRecordingText(damaged: DamagedRecording): string {
  if (damaged.parts === 0) return 'Damaged or modified recording';
  const range = damaged.firstHour === null || damaged.lastHour === null ? '' : ` · ${hours(damaged.firstHour, damaged.lastHour)}`;
  return `Damaged or modified recording · ${damaged.parts} ${damaged.parts === 1 ? 'part' : 'parts'} couldn’t be read${range}`;
}

type NoticeProps = { partial?: PartialRecording | null; damaged?: DamagedRecording | null };

export function RecordingNotice({ partial, damaged, path }: NoticeProps & { path?: string }) {
  const text = [damaged && damagedRecordingText(damaged), partial && partialRecordingText(partial)].filter(Boolean).join(' · ');
  return text ? <div className="notice warning shell-recording-notice" role="status">
    <Icon name="alert" /><span>{path && <><strong>{recordingDisplayName(path)}</strong> · </>}{text}</span>
  </div> : null;
}

export function PartialRecordingBadge({ partial, damaged }: NoticeProps) {
  const tooltip = [damaged && damagedRecordingText(damaged), partial && partialRecordingText(partial)].filter(Boolean).join('\n');
  return damaged || partial ? <span className="pill warning" data-tooltip={tooltip}>{damaged ? 'Damaged' : 'Partial'}</span> : null;
}
