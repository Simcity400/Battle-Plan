import type { DamagedRecording, PartialRecording } from '../generated/dashboard';
import { formatDateTime } from '../dashboard/calendar';
import { recordingDisplayName, recordingNameTooltip, useRecordingNames } from '../dashboard/recordingNames';

export function partialRecordingText(partial: PartialRecording): string {
  const failed = partial.failedHour === null ? '' : ` · ${formatDateTime(partial.failedHour)} not recorded`;
  return `Partial recording · ${partial.reason}${failed}`;
}

export function damagedRecordingText(damaged: DamagedRecording): string {
  // A failed checksum alone proves the file changed, not which parts; later blocks are
  // only read when playback reaches them, so claim nothing about what is missing.
  if (damaged.parts === 0) return 'Damaged or modified recording';
  const range = damaged.firstHour === null || damaged.lastHour === null ? ''
    : ` · ${formatDateTime(damaged.firstHour)} – ${formatDateTime(damaged.lastHour)}`;
  return `Damaged or modified recording · ${damaged.parts} ${damaged.parts === 1 ? 'part' : 'parts'} couldn't be read${range}`;
}

type NoticeProps = { partial?: PartialRecording | null; damaged?: DamagedRecording | null };

/** The loaded recording's notice stays visible through playback and tab changes. */
export function RecordingNotice({ partial, damaged, path }: NoticeProps & { path?: string }) {
  const names = useRecordingNames();
  const text = [damaged && damagedRecordingText(damaged), partial && partialRecordingText(partial)].filter(Boolean).join(' · ');
  return text ? <div className="notice warning shell-recording-notice" role="status">
    <span>{path && <><strong data-tooltip={recordingNameTooltip(path, names)}>{recordingDisplayName(path, names)}</strong> · </>}{text}</span>
  </div> : null;
}

export function PartialRecordingBadge({ partial, damaged }: NoticeProps) {
  const tooltip = [damaged && damagedRecordingText(damaged), ...(damaged?.reasons ?? []),
    partial && partialRecordingText(partial)].filter(Boolean).join('\n');
  return damaged || partial ? <span className="pill warning" data-tooltip={tooltip}>{damaged ? 'Damaged' : 'Partial'}</span> : null;
}
