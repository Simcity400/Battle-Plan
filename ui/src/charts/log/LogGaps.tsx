import type { MissingSpan } from '../../generated/dashboard';
import { dayLabel } from '../draw';
import { missingNote, missingThrough } from './logSource.ts';
import './log.css';

/** The hours a log list has reached that its log was not recorded whole, marked as not recorded: each span as a plain
 * sentence (`subject` and its dates), the recording's reason in its tooltip. Shown beside the list, never as hours
 * without rows. `until` is the list's cut (null: the whole span). */
export default function LogGaps({ missing, until, subject }: { missing: readonly MissingSpan[]; until: number | null; subject?: string }) {
  const note = missingNote(missingThrough(missing, until), dayLabel, subject);
  if (!note) return null;
  return <div className="log-gaps" role="note" aria-label="Hours not recorded">
    <b>Not recorded</b>
    <ul>{note.spans.map(span => <li key={span.text + span.detail} data-tooltip={span.detail}>{span.text}<span className="sr-only"> {span.detail}</span></li>)}</ul>
    <small>{note.counted}</small>
  </div>;
}
