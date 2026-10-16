import { dayLabel } from '../../charts/draw';
import type { EventBadge } from '../../charts/badges';
import type { usePreparedParty } from './preparedParty';
import { PreparedLogCoverage } from '../../charts/log/LogList';

/** One reached cluster page; every page remains reachable without accumulating it. */
export default function PartyEventList({ prepared, badges }: { prepared: ReturnType<typeof usePreparedParty>; badges: readonly EventBadge[] }) {
  if (!prepared.page) return null;
  return <div aria-label="Party events">
    <PreparedLogCoverage coverage={prepared.events?.coverage} />
    <button type="button" onClick={prepared.closePage}>Close events</button>
    {prepared.events ? <><ul>{badges.map((badge,index)=><li key={index}>{dayLabel(badge.hour)}: {badge.label}</li>)}</ul>
      <button type="button" disabled={prepared.page.cursor===prepared.page.start} onClick={prepared.previousPage}>Previous events</button>
      <button type="button" disabled={!prepared.events.cursor} onClick={prepared.nextPage}>Next events</button></> : <span>Loading events…</span>}
  </div>;
}
