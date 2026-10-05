import type { PartialRecording } from '../generated/dashboard';
import { formatDateTime } from '../dashboard/calendar';

export function partialRecordingText(partial: PartialRecording): string {
  const failed = partial.failedHour === null ? '' : ` · ${formatDateTime(partial.failedHour)} not recorded`;
  return `Partial recording · ${partial.reason}${failed}`;
}

/** The loaded recording's terminal notice stays visible through playback and tab changes. */
export function RecordingNotice({ partial }: { partial?: PartialRecording | null }) {
  return partial ? <div className="notice warning shell-recording-notice" role="status">
    <span>{partialRecordingText(partial)}</span>
  </div> : null;
}

export function PartialRecordingBadge({ partial }: { partial?: PartialRecording | null }) {
  return partial ? <span className="pill warning" data-tooltip={partialRecordingText(partial)}>Partial</span> : null;
}
