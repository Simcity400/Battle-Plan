import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { ResearchHistory, ResearchSlotHistory } from './types';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products.ts';
import WidgetState from '../../components/WidgetState';
import { validateResearchBreakdown, withBreakdown } from './researchData';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar';
import { Icon } from '../../components/Icons';
import { researchNumber } from './researchBreakdown';
import { completedResearchJourney, researchAssignments, researchJourney, researchTimeSplit, type ResearchTimeSplit } from './researchJourney';
import { researchTimeRows, type ResearchSegment } from './researchTimeBar';
import type { ResearchBreakdownPart, ResearchBreakdownReply, ResearchEntry } from '../../generated/dashboard';

const date = (hour: number) => { const value = gameDate(hour); return value ? formatDate(value) : '—'; };
const amount = (raw: bigint) => researchNumber(raw < 0n ? -raw : raw);
const days = (raw: bigint) => { const text = amount(raw); return `${text} ${text === '1' ? 'day' : 'days'}`; };

function TimeBar({ split, active, daysLeft }: { split: ResearchTimeSplit; active: boolean; daysLeft: number | null }) {
  const [hoveredKey, setHoveredKey] = useState<string | null>(null);
  const rows = useMemo(() => researchTimeRows(split, active, daysLeft), [split, active, daysLeft]);
  const hovered = hoveredKey === null || !rows ? null : [...rows.expected, ...rows.took].find(s => s.key === hoveredKey) ?? null;
  if (hoveredKey !== null && !hovered) setHoveredKey(null);
  if (!rows) return null;
  const { expected: shown, took, expectedTotal, elapsed, axis } = rows;
  const shownTotal = shown.reduce((sum, s) => sum + s.value, 0n);
  const tookTotal = took.reduce((sum, s) => sum + s.value, 0n);
  const width = (value: bigint) => `${Number(value * 100_000n / axis) / 1000}%`;
  const difference = expectedTotal - elapsed - split.remaining;
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
    <p className="research-time-readout" aria-hidden="true">
      {hovered ? <><span className={`research-time-swatch is-${hovered.tone}`} style={{ '--shade': hovered.index ?? 0 } as CSSProperties} /><span className="research-time-name">{hovered.name}</span><small>{hovered.kind}</small><strong>{hovered.effect}</strong></>
        : active ? <span>{rows.left === null ? `${days(split.remaining)} of base research time still to do` : `${days(rows.left)} left at the current research speed`}</span>
          : difference === 0n ? <span>As expected</span>
            : <><span>{difference > 0n ? 'Faster' : 'Slower'}</span><strong data-tone={difference > 0n ? 'faster' : 'slower'}>{difference > 0n ? '−' : '+'}{days(difference)}</strong></>}
    </p>
  </div>;
}

export function breakdownParts(slots: readonly ResearchSlotHistory[], slot: ResearchSlotHistory, entry: ResearchEntry, asked: number): ResearchBreakdownPart[] {
  return researchAssignments(slots, slot, entry).map(part => ({ slot: part.slot.index, firstObservedHour: part.entry.firstObservedHour,
    lastHour: part.entry.endedByHour ?? Math.max(part.entry.firstObservedHour, asked),
    started: part.entry.startedAfterHour !== null }));
}

export default function ResearchExplanation({ client, track, tag, hour, history, slots, slot, entry, description, daysLeft = null }: {
  client: DashboardClient; track: BoundTrack; tag: string; hour: number; daysLeft?: number | null;
  history: ResearchHistory; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; description?: string | null;
}) {
  const parts = breakdownParts(slots, slot, entry, hour);
  const request = { sourceId: track.source.id, revision: track.source.revision, product: 'research-breakdown', params: { branchId: track.branch.id, country: countryFromKey(tag), parts } };
  const { data, error, retry } = useProduct<ResearchBreakdownReply>(client, request, true, validateResearchBreakdown, track.source);
  const joined = useMemo<ResearchHistory>(() => ({ coverage: history.coverage, slots: withBreakdown(history.slots, data) }), [history, data]);
  const joinedSlots = useMemo(() => withBreakdown(slots, data), [slots, data]);
  const joinedSlot = joinedSlots.find(owner => owner.index === slot.index) ?? slot;
  const completed = useMemo(() => data ? completedResearchJourney(joined, slot.index, entry) : null,
    [data, joined, slot.index, entry.firstObservedHour, entry.technologyHandle, entry.level, entry.token,
      entry.outcome, entry.endedByHour, entry.outcomeObservedHour]);
  const journey = useMemo(() => completed ?? researchJourney(joinedSlots, joinedSlot, entry), [completed, joinedSlots, joinedSlot, entry]);
  const active = entry.outcome === 'active';
  const split = useMemo(() => data ? researchTimeSplit(journey, active) : null, [data, journey, active]);
  const missing = data?.parts.flatMap(part => part.missing ?? []) ?? [];
  return <div className="research-description">
    <div className="research-when">
      {entry.outcome === 'completed' && entry.endedByHour !== null
        ? <span data-tooltip={formatDateTime(entry.endedByHour)}><small>Completed</small>{date(entry.endedByHour)}</span>
        : <span data-tooltip={journey.startKnown ? formatDateTime(journey.firstHour) : 'Start date unknown.'}>
          <small>{journey.startKnown ? 'Started' : 'Started by'}</small>{date(journey.firstHour)}</span>}
    </div>
    {error && !data ? <WidgetState kind="error" failure={error} onRetry={retry} />
      : !data ? <p className="research-unavailable" role="status">Loading research time breakdown…</p>
        : split ? <TimeBar split={split} active={active} daysLeft={daysLeft} />
          : <p className="research-unavailable">Research time breakdown unavailable{missing.length ? `: ${missing.join(' ')}` : '.'}</p>}
    {description && <details className="research-description-text"><summary>Description<Icon name="chevron-down" className="research-chevron" aria-hidden="true" /></summary><p>{description}</p></details>}
  </div>;
}
