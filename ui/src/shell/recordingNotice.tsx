import type { DamagedRecording, PartialRecording } from '../generated/dashboard';
import { formatDateTime } from '../dashboard/calendar';
import { recordingDisplayName, recordingFilename } from '../dashboard/recordingNames';

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
  const text = [damaged && damagedRecordingText(damaged), partial && partialRecordingText(partial)].filter(Boolean).join(' · ');
  return text ? <div className="notice warning shell-recording-notice" role="status">
    <span>{path && <><strong data-tooltip={recordingFilename(path)}>{recordingDisplayName(path)}</strong> · </>}{text}</span>
  </div> : null;
}

/** The game build warnings as one pill in the header row, every one in full on hover and in
 * Map diagnostics: they change nothing on the dashboard, so they take none of its height. */
export function BuildNotice({ warnings }: { warnings?: string[] }) {
  if (!warnings?.length) return null;
  const text = warnings.join('\n');
  return <span className="pill warning shell-build-notice" role="status" tabIndex={0} aria-label={text} data-tooltip={text}>
    {warnings.length > 1 ? `Game build · ${warnings.length} notes` : 'Game build'}
  </span>;
}

export function PartialRecordingBadge({ partial, damaged }: NoticeProps) {
  const tooltip = [damaged && damagedRecordingText(damaged), ...(damaged?.reasons ?? []),
    partial && partialRecordingText(partial)].filter(Boolean).join('\n');
  return damaged || partial ? <span className="pill warning" data-tooltip={tooltip}>{damaged ? 'Damaged' : 'Partial'}</span> : null;
}
