import { useEffect, useMemo, useRef, useState } from 'react';
import { Select } from '../../components/Select';
import { VirtualList } from '../../components/VirtualList';
import { dayLabel } from '../../charts/draw';
import { formatFixed } from '../../charts/valueAxis';
import { HOUR_EPOCH } from '../../dashboard/calendar';
import { useProduct } from '../../dashboard/products';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import type { ReactNode } from 'react';
import type { FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';

const statusName = (status: number) => ['Active', 'Completed', 'Cancelled', 'Completing', 'Cancelling'][status] ?? `State ${status}`;
const title = (goal: FactionGoalDefinition) => goal.name || (goal.reason ? 'Goal name unavailable' : 'Unnamed goal');
const term = (name: string) => name.replaceAll('_', ' ');
// The stored invalid date formats as year -1, January 1 in the native calendar.
const UNSET_DATE = HOUR_EPOCH - 365 * 24;
const goalDateLabel = (hour: number) => hour === UNSET_DATE ? 'Unset' : dayLabel(hour);
const noInteraction = () => false;
function DetailList<T extends { ordinal: number }>({ items, label, owner, children }: { items: readonly T[] | null; label: string; owner: string | undefined; children: (item: T) => ReactNode }) {
  const scroll = useRef<HTMLDivElement>(null);
  return <section><h4>{label}</h4>{items === null ? <p>{label} unavailable.</p>
    : !items.length ? <p>None.</p> : <div ref={scroll} className="faction-goal-list">
      <VirtualList items={items} itemKey={item => String(item.ordinal)} estimate={60} identity={owner} label={label} scrollRef={scroll} isInteractive={noInteraction}>
        {item => <div className="faction-goal">{children(item)}</div>}
      </VirtualList>
    </div>}</section>;
}

function GoalPresentation({ definition }: { definition: FactionGoalDefinition }) {
  return <>
    {definition.flags.length > 0 && <span>{definition.flags.map(flag => `${term(flag.kind)}: ${flag.result === null ? 'Unavailable' : flag.result ? 'Yes' : 'No'}`).join(' | ')}</span>}
    {definition.conditions.map(condition => <div key={condition.kind}><span>{term(condition.kind)}: {condition.result === null ? 'Unavailable' : condition.result ? 'Met' : 'Not met'}</span>{condition.reasons.map(reason => <p key={reason}>{reason}</p>)}</div>)}
    {[...definition.progress, ...definition.sections, ...definition.rewards].map(reward => <div key={reward.kind}><strong>{term(reward.kind)}</strong>{reward.text && <p style={{ whiteSpace: 'pre-wrap' }}>{reward.text}</p>}{reward.reasons.map(reason => <p key={reason}>{reason}</p>)}</div>)}
  </>;
}

/** A selected faction's held state, dated independently of timeline preparation. */
export default function FactionDetails({ runtime, track, hour, active, factions, onDiagnostics }: {
  runtime: WidgetRuntime; track: BoundTrack; hour: number; active: boolean;
  factions: readonly { id: string; name: string }[];
  onDiagnostics: (warnings: string[]) => void;
}) {
  const [chosen, choose] = useState('');
  const [open, setOpen] = useState(false);
  const scroll = useRef<HTMLDivElement>(null);
  const selected = factions.some(faction => faction.id === chosen) ? chosen : factions[0]?.id;
  const request = selected && open ? { sourceId: track.source.id, revision: track.source.revision, product: 'faction-details', params: { branchId: track.branch.id, hour, faction: selected } } : null;
  const result = useProduct<FactionDetailReply>(runtime.client, request, active);
  const data = result.data;
  useEffect(() => { onDiagnostics(result.error ? [result.error] : data?.warnings ?? []); return () => onDiagnostics([]); }, [data, result.error, onDiagnostics]);
  const rows = useMemo(() => data ? [
    ...data.goals.map(goal => ({ id: `goal:${goal.ordinal}`, definition: goal.definition, state: statusName(goal.status), progress: goal.progressPercent, progressRaw: goal.progressRaw, sections: goal.activeSections, date: goal.date })),
    ...data.completed.map((definition, index) => ({ id: `completed:${index}`, definition, state: 'Completed history', progress: null, progressRaw: null, sections: null, date: null })),
    ...data.cancelled.map((definition, index) => ({ id: `cancelled:${index}`, definition, state: 'Cancelled history', progress: null, progressRaw: null, sections: null, date: null })),
  ] : [], [data]);
  if (!factions.length) return null;
  return <details className="faction-details" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>Faction details | {dayLabel(hour)}</summary>
    {open && <>
      <Select label="Faction details" value={selected ?? ''} options={factions.map(faction => ({ value: faction.id, label: faction.name }))} onChange={choose} />
      {result.error ? <p role="status">Faction details unavailable. <button type="button" onClick={result.retry}>Retry</button></p>
        : !data ? <p role="status">Loading faction details...</p> : <>
          <h4>Goals and manifest</h4>
          {data.goalReason ? <p>Goals unavailable: {data.goalReason}</p> : <>
          <div className="faction-manifest"><strong>Manifest: {data.manifest ? title(data.manifest) : 'None'}</strong>{data.manifest?.description && <p>{data.manifest.description}</p>}{data.manifest && <GoalPresentation definition={data.manifest} />}</div>
          {!!data.extraSlots.length && <p>Extra goal slots: {data.extraSlots.map((value, index) => `${['Short term', 'Medium term', 'Long term'][index] ?? `Term ${index + 1}`} ${value}`).join(' | ')}</p>}
          {!!data.termDates.length && <p>Term dates: {data.termDates.map(goalDateLabel).join(' | ')}</p>}
          {!rows.length ? <p>No goals.</p> : <div ref={scroll} className="faction-goal-list">
            <VirtualList items={rows} itemKey={row => row.id} estimate={100} identity={selected} label="Faction goals" scrollRef={scroll} isInteractive={noInteraction}>
              {row => <div className="faction-goal">
                <strong>{title(row.definition)}</strong><span>{row.state}{row.definition.category ? ` | ${term(row.definition.category)}` : ''}</span>
                {row.progressRaw !== null && <span>Held progress: {formatFixed(row.progressRaw, 3, 3)}%</span>}
                {row.sections !== null && <span>Held active sections: {row.sections.length ? row.sections.join(', ') : 'None'}</span>}
                {row.date != null && <span>Goal date: {goalDateLabel(row.date)}</span>}
                {row.definition.description && <p>{row.definition.description}</p>}
                <GoalPresentation definition={row.definition} />
              </div>}
            </VirtualList>
          </div>}
          </>}
          <p>Faction research: {data.researchUnlocked === null ? 'Unavailable' : data.researchUnlocked ? 'Unlocked' : 'Locked'} | Faction military access: {data.manpowerUnlocked === null ? 'Unavailable' : data.manpowerUnlocked ? 'Unlocked' : 'Locked'}</p>
          <DetailList owner={selected} items={data.rules} label="Rules">{rule => <>
            <strong>{rule.name ?? rule.token ?? 'Rule name unavailable'}</strong>
            {rule.reason && <span>{rule.reason}</span>}
          </>}</DetailList>
          <DetailList owner={selected} items={data.programmes} label={data.programmesReadHour == null ? 'Shared programmes' : `Shared programmes | Details read ${dayLabel(data.programmesReadHour)}`}>{programme => <>
            <strong>{programme.owner ?? 'Country unavailable'}</strong>
            <span>{programme.activeProject === false ? 'No active project' : programme.project ?? 'Project unavailable'}</span>
            {programme.reason && <span>{programme.reason}</span>}
          </>}</DetailList>
          {data.manpowerTotal !== null && <p>Manpower pool: {data.manpowerTotal.toLocaleString()}</p>}
          <DetailList owner={selected} items={data.manpower} label="Manpower contributions">{entry => <>
            <strong>{entry.tag ?? 'Country unavailable'}</strong><span>{entry.value.toLocaleString()}</span>
          </>}</DetailList>
          <DetailList owner={selected} items={data.minimumUpgrades?.map(upgrade => ({ ...upgrade, ordinal: upgrade.group })) ?? null} label="Faction minimum upgrades">{upgrade => <><strong>{upgrade.name ?? upgrade.token ?? 'Unavailable'} (group {upgrade.group})</strong>{upgrade.reason && <span>{upgrade.reason}</span>}</>}</DetailList>
          <DetailList owner={selected} items={data.doctrineFolders} label="Doctrine sharing folders">{folder => <><strong>{folder.name ?? folder.token ?? 'Unavailable'}</strong>{folder.reason && <span>{folder.reason}</span>}</>}</DetailList>
          <section><h4>Power projection and influence totals</h4>
            <p>Projection: {data.projectionRaw === null ? 'Unavailable' : formatFixed(data.projectionRaw, 5, 5)} | Influence: {data.influenceTotalRaw === null ? 'Unavailable' : formatFixed(data.influenceTotalRaw, 5, 5)}</p>
            <p>Projection from effects: {data.projectionEffectsRaw === null ? 'Unavailable' : formatFixed(data.projectionEffectsRaw, 5, 5)}</p>
          </section>
          <DetailList owner={selected} items={data.resources} label="Extracted resources">{entry => <><strong>{entry.name ?? entry.token ?? `Resource slot ${entry.ordinal}`}</strong><span>{formatFixed(entry.raw, 5, 5)}</span>{entry.reason && <span>{entry.reason}</span>}</>}</DetailList>
          <DetailList owner={selected} items={data.systemResources} label="Faction system extracted resources">{entry => <><strong>{entry.name ?? entry.token ?? `Resource slot ${entry.ordinal}`}</strong><span>{formatFixed(entry.raw, 5, 5)}</span>{entry.reason && <span>{entry.reason}</span>}</>}</DetailList>
          <section><h4>Research sharing</h4>
            <p>Scientist base: {data.researchBaseRaw === null ? 'Unavailable' : formatFixed(data.researchBaseRaw, 5, 5)}</p>
            <p>Sharing members: {data.researchCountries === null ? 'Unavailable' : data.researchCountries.join(' | ') || 'None'}</p>
            <p>Research bonuses: {data.researchBonuses === null ? 'Unavailable' : data.researchBonuses.map(raw => formatFixed(raw, 5, 5)).join(' | ') || 'None'}</p>
          </section>
          <DetailList owner={selected} items={data.accounts} label="Member accounting">{account => <>
            <strong>{account.tag ?? `Country ${account.country}`} | {account.currentMember ? 'Current member' : 'Retained account'} | Account {account.ordinal}</strong>
            <span>Influence terms (fixed point): {account.influence.map(raw => formatFixed(raw, 5, 5)).join(' | ')}</span>
            <span>Power projection terms: {account.projection.map(raw => formatFixed(raw, 5, 5)).join(' | ')} | Rank: {account.projectionRank}</span>
            <span>Influence share: {formatFixed(account.influenceShareRaw, 3, 3)}% | Projection share: {formatFixed(account.projectionShareRaw, 3, 3)}%</span>
            <span>Initiative: {formatFixed(account.initiativeRaw, 5, 5)} | Contribution: {formatFixed(account.contributionRaw, 5, 5)}</span>
            <span>Contribution gain: {account.contributionGain.map(raw => formatFixed(raw, 5, 5)).join(' | ')}</span>
            <span>Previous war score: {formatFixed(account.previousWarScoreRaw, 5, 5)}</span>
            <span>Selected upgrades: {account.upgrades.length ? account.upgrades.map(upgrade => `${upgrade.name ?? upgrade.token ?? 'Unavailable'} (group ${upgrade.group})`).join(' | ') : 'None'}</span>
            {account.upgrades.map(upgrade => upgrade.reason && <span key={upgrade.group}>{upgrade.reason}</span>)}
            <span>Completed goals: {account.completed.length ? account.completed.map(title).join(' | ') : 'None'}</span>
          </>}</DetailList>
          {data.variableSeed !== null && <p>Variable seed: {data.variableSeed}</p>}
          <DetailList owner={selected} items={data.variables} label="Faction variables">{entry => <><strong>{entry.name || '(empty key)'}</strong><span>{formatFixed(entry.valueRaw, 5, 5)}</span></>}</DetailList>
          {data.presentations.map(detail => <section key={detail.kind}><h4>{detail.kind}</h4>{detail.text && <p style={{ whiteSpace: 'pre-wrap' }}>{detail.text}</p>}{detail.reasons.map(reason => <p key={reason}>{reason}</p>)}</section>)}
          <section className="faction-detail-coverage"><h4>Unavailable facts</h4>{data.refusals.map(refusal => <p key={refusal.fact}><strong>{term(refusal.fact)} ({refusal.count})</strong>: {refusal.reason}</p>)}</section>
        </>}
    </>}
  </details>;
}
