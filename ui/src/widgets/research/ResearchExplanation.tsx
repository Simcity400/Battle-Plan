import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { ResearchEntry, ResearchHistory, ResearchSlotHistory } from '../../generated/dashboard';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar';
import { Icon } from '../../components/Icons';
import { researchNumber } from './researchBreakdown';
import { completedResearchJourney, researchJourney, researchTimeSplit, type ResearchTimeSplit } from './researchJourney';
import { researchTimeRows, type ResearchSegment } from './researchTimeBar';

const date = (hour: number) => { const value = gameDate(hour); return value ? formatDate(value) : '—'; };
const amount = (raw: bigint) => researchNumber(raw < 0n ? -raw : raw);
const days = (raw: bigint) => { const text = amount(raw); return `${text} ${text === '1' ? 'day' : 'days'}`; };

/** Two rows on one time axis (see `researchTimeRows`). Hovering a segment
 * highlights it and names it underneath. */
function TimeBar({ split, active }: { split: ResearchTimeSplit; active: boolean }) {
  // The key only: playback replaces the segments, and a gone one ends the hover.
  const [hoveredKey, setHoveredKey] = useState<string | null>(null);
  const rows = useMemo(() => researchTimeRows(split, active), [split, active]);
  const hovered = hoveredKey === null || !rows ? null : [...rows.expected, ...rows.took].find(s => s.key === hoveredKey) ?? null;
  if (hoveredKey !== null && !hovered) setHoveredKey(null);
  if (!rows) return null;
  const { expected: shown, took, expectedTotal, elapsed, axis } = rows;
  const shownTotal = shown.reduce((sum, s) => sum + s.value, 0n);
  const tookTotal = took.reduce((sum, s) => sum + s.value, 0n);
  const width = (value: bigint) => `${Number(value * 100_000n / axis) / 1000}%`;
  const difference = expectedTotal - elapsed - split.remaining;
  const segment = (s: ResearchSegment) => <span key={s.key} className={`is-${s.tone}${hovered?.key === s.key ? ' is-hovered' : ''}`}
    style={{ flexGrow: Number(s.value), '--shade': s.index ?? 0 } as CSSProperties}
    onPointerEnter={() => setHoveredKey(s.key)} onPointerLeave={() => setHoveredKey(current => current === s.key ? null : current)} />;
  const summary = [...shown, ...took].map(s => `${s.name}${s.kind ? ` (${s.kind})` : ''}: ${s.effect}`).join('; ');
  return <div className={`research-time${hovered ? ' is-hovering' : ''}`} role="img" aria-label={summary}>
    <span className="research-time-label">Expected</span>
    <span className="research-time-track"><span className="research-time-row" style={{ width: width(shownTotal) }}>{shown.map(segment)}</span></span>
    <span className="research-time-value">{amount(expectedTotal)} d</span>
    <span className="research-time-label">{active ? 'So far' : 'Took'}</span>
    <span className="research-time-track"><span className="research-time-row" style={{ width: width(tookTotal) }}>{took.map(segment)}</span></span>
    <span className="research-time-value">{amount(elapsed)} d</span>
    <p className="research-time-readout" aria-hidden="true">
      {hovered ? <><span className={`research-time-swatch is-${hovered.tone}`} style={{ '--shade': hovered.index ?? 0 } as CSSProperties} /><span className="research-time-name">{hovered.name}</span><small>{hovered.kind}</small><strong>{hovered.effect}</strong></>
        : active ? <span>{days(split.remaining)} of research left</span>
          : difference === 0n ? <span>As expected</span>
            : <><span>{difference > 0n ? 'Faster' : 'Slower'}</span><strong data-tone={difference > 0n ? 'faster' : 'slower'}>{difference > 0n ? '−' : '+'}{days(difference)}</strong></>}
    </p>
  </div>;
}

export default function ResearchExplanation({ history, slots, slot, entry, description }: {
  history: ResearchHistory; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; description?: string | null;
}) {
  // Playback recreates clipped entries every hour. Completed work depends only
  // on immutable source history and its already-visible observation bounds.
  const completed = useMemo(() => completedResearchJourney(history, slot.index, entry),
    [history, slot.index, entry.firstObservedHour, entry.technologyHandle, entry.level, entry.token,
      entry.outcome, entry.endedByHour, entry.outcomeObservedHour]);
  const journey = useMemo(() => completed ?? researchJourney(slots, slot, entry), [completed, slots, slot, entry]);
  const active = entry.outcome === 'active';
  const split = useMemo(() => researchTimeSplit(journey, active), [journey, active]);
  return <div className="focus-description research-description">
    <div className="research-when">
      {entry.outcome === 'completed' && entry.endedByHour !== null
        ? <span data-tooltip={formatDateTime(entry.endedByHour)}><small>Completed</small>{date(entry.endedByHour)}</span>
        : <span data-tooltip={journey.startKnown ? formatDateTime(journey.firstHour) : 'Start date unknown.'}>
          <small>{journey.startKnown ? 'Started' : 'Started by'}</small>{date(journey.firstHour)}</span>}
    </div>
    {split ? <TimeBar split={split} active={active} /> : <p className="research-unavailable">Research time breakdown unavailable.</p>}
    {description && <details className="research-description-text"><summary>Description<Icon name="chevron-down" className="research-chevron" aria-hidden="true" /></summary><p>{description}</p></details>}
  </div>;
}
