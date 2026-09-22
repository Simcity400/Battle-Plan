import { useMemo } from 'react';
import type { ResearchEntry, ResearchHistory, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';
import { formatDate, formatDateTime, gameDate } from '../../dashboard/calendar';
import { researchNumber } from './researchBreakdown';
import { completedResearchJourney, researchJourney, type ResearchJourneyPeriod, type ResearchCredit } from './researchJourney';
import { researchRateAt } from './researchData';

const labels: Record<string, string> = {
  country: 'Country', category: 'Technology', designTeam: 'Design team', experience: 'Experience',
  license: 'Licence', scientist: 'Scientist research', applied: 'Research bonus', sharing: 'Technology sharing',
};
const date = (hour: number) => { const value = gameDate(hour); return value ? formatDate(value) : '—'; };
const percent = (raw: string) => `${BigInt(raw) > 0n ? '+' : ''}${researchNumber(BigInt(raw) * 100n, 2)}%`;
const days = (hours: number) => researchNumber(BigInt(hours) * 100_000n / 24n);
const tone = (rate: bigint | null) => rate === null ? 'unknown' : rate > 100_000n ? 'faster' : rate < 100_000n ? 'slower' : 'normal';
const rateText = (rate: bigint | null) => rate === null ? '—' : `${researchNumber(rate, 2)}×`;

function Factors({ state, hour, atStart = false }: { state: ResearchSlotState; hour: number; atStart?: boolean }) {
  const modifiers = state.details?.modifiers;
  if (!modifiers) return <span className="research-unavailable">Missing modifier data</span>;
  const rate = researchRateAt(state, hour);
  return <div className="research-factors">
    <dl><div><dt>Base rate</dt><dd>1.00×</dd></div></dl>
    {modifiers.components.filter(c => c.raw !== '0' || c.sources.some(s => s.raw !== '0')).map(component =>
      <div className="research-factor" key={component.kind}>
        <dl><div><dt>{labels[component.kind]}</dt><dd data-tone={tone(100_000n + BigInt(component.raw))}>{percent(component.raw)}</dd></div></dl>
        {component.sources.some(s => s.raw !== '0') && <ul>{component.sources.filter(s => s.raw !== '0').map((source, i) => <li key={i}>
          <span>{source.name ?? source.token ?? labels[component.kind]}</span>
          <span data-tooltip={component.kind === 'scientist' ? 'Scientist efficiency; included in the aggregate above alongside skill and facility effects.' : undefined}>
            {percent(source.raw)}{component.kind === 'scientist' ? ' eff.' : ''}
          </span>
        </li>)}</ul>}
      </div>)}
    <dl className="research-rate-total">
      <div><dt>Combined bonus</dt><dd>{percent(modifiers.totalBonusRaw)}</dd></div>
      {rate && rate.penaltyRaw !== '0' && <div><dt>Ahead penalty</dt><dd>÷ {researchNumber(100_000n + BigInt(rate.penaltyRaw), 2)}</dd></div>}
      {rate && rate.rateRaw === modifiers.minimumSpeedRaw && <div><dt>Minimum rate</dt><dd>{rateText(BigInt(modifiers.minimumSpeedRaw))}</dd></div>}
      <div><dt>{atStart ? 'Rate at start' : 'Effective rate'}</dt><dd>{rateText(rate ? BigInt(rate.rateRaw) : null)}</dd></div>
    </dl>
    {(state.appliedDaysRaw !== '0' || (state.aheadReductionYearsRaw !== null && state.aheadReductionYearsRaw !== '0')) &&
      <dl className="research-adjustments">
        {state.appliedDaysRaw !== '0' && <div><dt>Saved days applied</dt><dd>{researchNumber(BigInt(state.appliedDaysRaw))} d</dd></div>}
        {state.aheadReductionYearsRaw !== null && state.aheadReductionYearsRaw !== '0' && <div><dt>Ahead reduction</dt><dd>{researchNumber(BigInt(state.aheadReductionYearsRaw))} yr</dd></div>}
      </dl>}
    {state.details?.bonuses?.map((bonus, i) => <div className="research-bonus-source" key={`${bonus.id}:${i}`}>
      <span>{bonus.name ?? bonus.token ?? 'Research bonus'}</span>
      <span>{[
        bonus.speedBonusRaw && bonus.speedBonusRaw !== '0' ? percent(bonus.speedBonusRaw) : null,
        bonus.costReductionRaw && bonus.costReductionRaw !== '0' ? `${researchNumber(BigInt(bonus.costReductionRaw) * 100n)}% cost reduction` : null,
        bonus.aheadReductionYearsRaw && bonus.aheadReductionYearsRaw !== '0' ? `${researchNumber(BigInt(bonus.aheadReductionYearsRaw))} yr ahead reduction` : null,
      ].filter(Boolean).join(' · ')}</span>
    </div>)}
  </div>;
}

function Period({ period, showSlot }: { period: ResearchJourneyPeriod; showSlot: boolean }) {
  const duration = period.endHour - period.firstHour;
  const average = period.work !== null ? period.work / BigInt(duration) : null;
  const changing = period.firstRate !== period.lastRate;
  return <details className="research-period" data-tone={period.paused ? 'normal' : tone(average)}>
    <summary>
      <span className="research-period-dates" data-tooltip={`${formatDateTime(period.firstHour)} → ${formatDateTime(period.endHour)}`}>
        {date(period.firstHour)} <span>→ {date(period.endHour)}</span>
      </span>
      <span className="research-period-metrics"><span>{days(duration)} d{showSlot && period.slotIndex !== null ? ` · Slot ${period.slotIndex + 1}` : ''}</span><strong>{period.paused ? 'Paused' : `${rateText(average)}${changing && average !== null ? ' avg' : ''}`}</strong><span aria-hidden="true">⌄</span></span>
    </summary>
    <div className="research-period-body">
      {period.work !== null && !period.paused && <div className="research-earned" data-tooltip="Estimated base research days earned during this period. Recalculated at each game-day boundary; recorded progress can differ because of update timing or immediate credits.">
        <strong>≈ {researchNumber(period.work / 24n)} d</strong><span>research progress</span>
      </div>}
      {changing && period.firstRate !== null && <div className="research-rate-range">{rateText(period.firstRate)} → {rateText(period.lastRate)}</div>}
      {period.state ? <Factors state={period.state} hour={period.firstHour} atStart={changing} /> : <span className="research-unavailable">{period.paused ? 'Not assigned to a research slot' : 'Recording gap'}</span>}
    </div>
  </details>;
}

function Credit({ credit }: { credit: ResearchCredit }) {
  const returned = credit.kind === 'returned';
  return <details className="research-credit" data-tone={credit.work < 0n ? 'slower' : 'faster'}>
    <summary><span>{credit.kind === 'adjusted' ? 'Saved credit adjusted' : credit.kind === 'existing' ? 'Existing saved credit' : `${researchNumber(credit.days < 0n ? -credit.days : credit.days)} d ${returned ? 'returned' : 'saved'}`}</span>
      <strong>{credit.work > 0n ? '+' : ''}{researchNumber(credit.work)} d <span aria-hidden="true">⌄</span></strong></summary>
    <div><span>{date(credit.hour)} · Slot {credit.slotIndex + 1}</span>
      <span>{returned ? 'Progress removed; saved days returned to the slot.' : credit.kind === 'existing' ? 'Already included when recording began.' : 'Progress credit at the stored assignment rate.'}</span>
    </div>
  </details>;
}

export default function ResearchExplanation({ history, slots, slot, entry, current, description }: {
  history: ResearchHistory; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; current: ResearchSlotState | null; description?: string | null;
}) {
  // Playback recreates clipped entries every hour. Completed work depends only
  // on immutable source history and its already-visible observation bounds.
  const completed = useMemo(() => completedResearchJourney(history, slot.index, entry),
    [history, slot.index, entry.firstObservedHour, entry.technologyHandle, entry.level, entry.token,
      entry.outcome, entry.endedByHour, entry.outcomeObservedHour]);
  const data = useMemo(() => completed ?? researchJourney(slots, slot, entry), [completed, slots, slot, entry]);
  const elapsed = Math.max(0, data.endHour - data.firstHour);
  const partial = !data.startKnown || !data.creditsComplete || entry.outcome === 'unknown';
  const full = data.complete && data.base !== null;
  const difference = full ? data.base! - BigInt(elapsed) * 100_000n / 24n : null;
  const progress = data.observed !== null && data.latestBase !== null && data.latestBase > 0n
    ? Number((data.observed * 10000n / data.latestBase).toString()) / 100 : null;
  const currentRate = current ? researchRateAt(current, entry.lastObservedHour) : null;
  const timeline = [
    ...data.periods.map(period => ({ hour: period.firstHour, order: 2, period, credit: null })),
    ...data.credits.map(credit => ({ hour: credit.hour, order: credit.kind === 'returned' ? 0 : 1, period: null, credit })),
  ].sort((a, b) => a.hour - b.hour || a.order - b.order);
  return <div className="focus-description research-description">
    <dl className="research-dates">
      <div><dt>Assigned</dt><dd data-tooltip={data.startedAfterHour === null ? 'Research began before the readable recording.' : `First assignment: ${formatDateTime(data.firstHour)}`}>
        {data.startedAfterHour === null ? 'By ' : ''}{date(data.firstHour)}</dd></div>
      <div><dt>Completed</dt><dd>{entry.outcome === 'completed' && entry.endedByHour !== null ? date(entry.endedByHour) : '—'}</dd></div>
    </dl>
    <dl className="research-time-cards">
      <div><dt>Base time</dt><dd data-tooltip="First recorded research cost, at the base rate of one research day per day.">{data.base !== null ? researchNumber(data.base) : '—'}<small> days</small></dd></div>
      <div><dt>{partial ? 'Time observed' : entry.outcome === 'completed' ? 'Time taken' : 'Time spent'}</dt><dd
        data-tooltip={partial ? 'Earlier progress or missing observations prevent a complete research duration.' : undefined}>
        {partial ? '≥ ' : '≈ '}{days(elapsed)}<small> days</small></dd></div>
    </dl>
    {difference !== null && <div className="research-time-difference" data-tone={tone(100_000n + difference)}>
      {difference === 0n ? 'Matches base time' : `${researchNumber(difference < 0n ? -difference : difference)} d ${difference > 0n ? 'shorter' : 'longer'} than base`}
    </div>}
    {current && <div className="research-current-summary">
      <span>{entry.outcome === 'active' ? 'In progress' : entry.outcome === 'completed' ? 'Completed' : 'Assignment ended'}</span>
      {entry.outcome === 'active' && <strong data-tooltip="Estimate at the recorded rate; future modifier changes and a declining ahead penalty can change it.">
        {current.details?.remainingDays != null ? `≈ ${current.details.remainingDays} d left` : 'Remaining —'}
      </strong>}
      {progress !== null && <progress value={Math.max(0, Math.min(100, progress))} max={100} aria-label="Recorded research progress" />}
    </div>}
    {current && entry.outcome === 'active' && <details className="research-current-rate">
      <summary><span>Current rate</span><strong>{rateText(currentRate ? BigInt(currentRate.rateRaw) : null)} <span aria-hidden="true">⌄</span></strong></summary>
      <Factors state={current} hour={entry.lastObservedHour} />
    </details>}
    {timeline.length > 0 && <div className="research-timeline">
      <div className="research-section-label">Research timeline</div>
      <div className="research-timeline-strip" aria-hidden="true">{data.periods.map(period => <span key={period.firstHour}
        data-tone={period.paused ? 'normal' : tone(period.work !== null ? period.work / BigInt(period.endHour - period.firstHour) : null)}
        style={{ flexGrow: period.endHour - period.firstHour }} />)}</div>
      {timeline.map((item, i) => item.period ? <Period key={`period:${item.hour}:${i}`} period={item.period} showSlot={data.parts.length > 1} />
        : item.credit && <Credit key={`credit:${item.hour}:${i}`} credit={item.credit} />)}
    </div>}
    {data.comparison && <details className="research-reconciliation">
      <summary>Progress check</summary>
      <dl>
        <div><dt>Starting progress</dt><dd>{researchNumber(data.initial!)} d</dd></div>
        <div><dt>Recorded gain</dt><dd>{researchNumber(data.comparison.observed)} d</dd></div>
        {data.comparison.creditGain !== 0n && <>
          <div><dt>Research over time</dt><dd>≈ {researchNumber(data.comparison.rateGain)} d</dd></div>
          <div><dt>Saved-days change</dt><dd>{researchNumber(data.comparison.creditGain)} d</dd></div>
        </>}
        <div><dt>Calculated gain</dt><dd>≈ {researchNumber(data.comparison.calculated)} d</dd></div>
        {data.comparison.difference !== 0n && <div><dt>Difference</dt><dd data-tooltip="Unreconciled difference. Daily update timing, immediate progress credits, or missing formula inputs can affect this; it is not assigned to a modifier.">{researchNumber(data.comparison.difference)} d</dd></div>}
      </dl>
    </details>}
    {data.costChanged && <span className="research-unavailable">Base cost changed during research</span>}
    {description && <details className="research-description-text"><summary>Description</summary><p>{description}</p></details>}
  </div>;
}
