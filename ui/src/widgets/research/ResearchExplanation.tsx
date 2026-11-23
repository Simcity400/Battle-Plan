import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { ResearchHistory, ResearchSlotHistory } from './types';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products.ts';
import WidgetState from '../../components/WidgetState';
import { completedBy, validateResearchBreakdown, validateResearchSlots, withBreakdown } from './researchData';
import { slotNumbers } from './researchSlots';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar';
import { Icon } from '../../components/Icons';
import { researchNumber } from './researchBreakdown';
import { completedResearchJourney, researchAssignments, researchJourney, researchTimeSplit, type ResearchTimeSplit } from './researchJourney';
import { researchDaysLeftMissing, researchTimeRows, type ResearchDaysLeft, type ResearchSegment } from './researchTimeBar';
import type { ResearchBreakdownPart, ResearchBreakdownReply, ResearchEntry, ResearchSlotsReply } from '../../generated/dashboard';

const date = (hour: number) => { const value = gameDate(hour); return value ? formatDate(value) : '—'; };
const amount = (raw: bigint) => researchNumber(raw < 0n ? -raw : raw);

function TimeBar({ split, active, daysLeft }: { split: ResearchTimeSplit; active: boolean; daysLeft: ResearchDaysLeft | null }) {
  const [hoveredKey, setHoveredKey] = useState<string | null>(null);
  const rows = useMemo(() => researchTimeRows(split, active, daysLeft), [split, active, daysLeft]);
  const hovered = hoveredKey === null || !rows ? null : [...rows.expected, ...rows.took].find(s => s.key === hoveredKey) ?? null;
  if (hoveredKey !== null && !hovered) setHoveredKey(null);
  if (!rows) return null;
  const { expected: shown, took, expectedTotal, elapsed, axis } = rows;
  const shownTotal = shown.reduce((sum, s) => sum + s.value, 0n);
  const tookTotal = took.reduce((sum, s) => sum + s.value, 0n);
  const width = (value: bigint) => `${Number(value * 100_000n / axis) / 1000}%`;
  const segment = (s: ResearchSegment) => <span key={s.key} className={`is-${s.tone}${hovered?.key === s.key ? ' is-hovered' : ''}`}
    data-tooltip={`${s.name}${s.kind ? ` (${s.kind})` : ''}: ${s.effect}`}
    style={{ flexGrow: Number(s.value), '--shade': s.index ?? 0 } as CSSProperties}
    onPointerEnter={() => setHoveredKey(s.key)} onPointerLeave={() => setHoveredKey(current => current === s.key ? null : current)} />;
  const summary = [...shown, ...took].map(s => `${s.name}${s.kind ? ` (${s.kind})` : ''}: ${s.effect}`).join('; ');
  return <div className={`research-time${hovered ? ' is-hovering' : ''}`} role="img" aria-label={summary}>
    <span className="research-time-label">Expected</span>
    <span className="research-time-track"><span className="research-time-row" style={{ width: width(shownTotal) }}>{shown.map(segment)}</span></span>
    <span className="research-time-value">{amount(expectedTotal)} d</span>
    <span className="research-time-label">Actual</span>
    <span className="research-time-track"><span className="research-time-row" style={{ width: width(tookTotal) }}>{took.map(segment)}</span></span>
    <span className="research-time-value">{amount(elapsed)} d</span>
  </div>;
}

export function breakdownParts(slots: readonly ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry, asked: number): ResearchBreakdownPart[] {
  return researchAssignments(slots, slot, entry).map(part => ({ slot: part.slot.index, firstObservedHour: part.entry.firstObservedHour,
    lastHour: part.entry.endedByHour ?? Math.max(part.entry.firstObservedHour, asked),
    started: part.entry.startedAfterHour !== null }));
}

export function daysLeftAt(reply: ResearchSlotsReply, slot: number, entry: ResearchEntry, hour: number): ResearchDaysLeft | null {
  const read = reply.slots.find(item => item.index === slot && item.state.token === entry.token && item.state.databaseIndex === entry.databaseIndex
    && item.state.entryFirstObservedHour === entry.firstObservedHour);
  const details = read?.state.details;
  const left = read ? slotNumbers(read, hour).left : null;
  if (left === null || details?.remainingDays == null || details.estimateRaw === null) return null;
  return { left, rounding: BigInt(details.remainingDays) * 100_000n - BigInt(details.estimateRaw) };
}

export default function ResearchExplanation({ client, track, tag, hour, history, slots, slot, entry, description }: {
  client: DashboardClient; track: BoundTrack; tag: string; hour: number;
  history: ResearchHistory; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; description?: string | null;
}) {
  const parts = breakdownParts(slots, slot, entry, hour);
  const country = countryFromKey(tag);
  const bounded = entry.outcome === 'completed' && completedBy(entry);
  const request = bounded ? null : { sourceId: track.source.id, revision: track.source.revision, product: 'research-breakdown', params: { branchId: track.branch.id, country, parts } };
  const { data, error, retry } = useProduct<ResearchBreakdownReply>(client, request, true, validateResearchBreakdown, track.source);
  const active = entry.outcome === 'active';
  const estimate = useProduct<ResearchSlotsReply>(client, active ? { sourceId: track.source.id, revision: track.source.revision, product: 'research-slots',
    params: { branchId: track.branch.id, country, hour } } : null, true, validateResearchSlots, track.source);
  const daysLeft = estimate.data ? daysLeftAt(estimate.data, slot.index, entry, hour) : null;
  const joined = useMemo<ResearchHistory>(() => ({ coverage: history.coverage, slots: withBreakdown(history.slots, data) }), [history, data]);
  const joinedSlots = useMemo(() => withBreakdown(slots, data), [slots, data]);
  const joinedSlot = joinedSlots.find(owner => owner.index === slot.index) ?? slot;
  const completed = useMemo(() => data ? completedResearchJourney(joined, slot.index, entry) : null,
    [data, joined, slot.index, entry.firstObservedHour, entry.technologyHandle, entry.level, entry.token,
      entry.outcome, entry.endedByHour, entry.outcomeObservedHour]);
  const journey = useMemo(() => completed ?? researchJourney(joinedSlots, joinedSlot, entry), [completed, joinedSlots, joinedSlot, entry]);
  const split = useMemo(() => data ? researchTimeSplit(journey, active) : null, [data, journey, active]);
  const missing = data?.parts.flatMap(part => part.missing ?? []) ?? [];
  return <div className="research-description">
    <div className="research-when">
      {entry.outcome === 'completed' && entry.endedByHour !== null
        ? <span data-tooltip={formatDateTime(entry.endedByHour)}><small>{completedBy(entry) ? 'Completed by' : 'Completed'}</small>{date(entry.endedByHour)}</span>
        : <span data-tooltip={journey.startKnown ? formatDateTime(journey.firstHour) : 'Start date unknown.'}>
          <small>{journey.startKnown ? 'Started' : 'Started by'}</small>{date(journey.firstHour)}</span>}
      {active && <span data-tooltip={formatDateTime(hour)}><small>As of</small>{date(hour)}</span>}
    </div>
    {bounded ? <p className="research-unavailable">Research time breakdown unavailable: the research is known only to have completed by {date(entry.endedByHour!)}.</p>
      : (error || active && estimate.error) && !(data && (!active || estimate.data))
      ? <WidgetState kind="error" failure={(error ?? estimate.error)!} onRetry={error ? retry : estimate.retry} />
      : !data || active && !estimate.data ? <p className="research-unavailable" role="status">Loading research time breakdown…</p>
        : split && researchDaysLeftMissing(split, active, daysLeft)
          ? <p className="research-unavailable">Research time breakdown unavailable: the game's days-left estimate is not recorded at this hour.</p>
        : split ? <TimeBar split={split} active={active} daysLeft={daysLeft} />
          : <p className="research-unavailable">Research time breakdown unavailable{missing.length ? `: ${missing.join(' ')}` : '.'}</p>}
    {description && <details className="research-description-text"><summary>Description<Icon name="chevron-down" className="research-chevron" aria-hidden="true" /></summary><p>{description}</p></details>}
  </div>;
}
