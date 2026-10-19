import { useState } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { dayLabel } from '../../charts/draw';
import { fixed5, validateWargoalsReply } from './warGoalsData';
import type { WargoalTerms } from '../../generated/dashboard';

type Runtime = WidgetProps['runtime'];
type Track = Runtime['tracks'][number];
const EMPTY: string[] = [];
function Terms({ value }: { value: WargoalTerms | null }) {
  return value ? <>
    <p>Annexation: {value.annex ? 'yes' : 'no'}; default expiry: {value.expiryDays} days</p>
    <p>Generation cost: {value.baseGenerationCost} base + {value.perStateGenerationCost} per state; world tension: {fixed5(value.threat)}</p>
    <p>Allowed conditions</p>
    <pre>{value.allowed === null ? 'Description unavailable.' : value.allowed || 'No authored conditions.'}</pre>
    <p>Available conditions</p>
    <pre>{value.available === null ? 'Description unavailable.' : value.available || 'No authored conditions.'}</pre>
    {value.peace.map(term => <p key={term.name}>{term.name}: limit {term.limit}; cost ×{fixed5(term.costFactor)}; tension ×{fixed5(term.threatFactor)}{term.conditioned ? '; condition result unavailable' : ''}</p>)}
  </> : <p>Goal terms unavailable.</p>;
}
export default function WarGoalsPanel({ runtime, track, tag, hour, active, expanded = false, onWarnings }: {
  runtime: Runtime; track: Track; tag: string | null; hour: number; active: boolean; expanded?: boolean; onWarnings: (notes: string[]) => void;
}) {
  const [open, setOpen] = useState(expanded);
  const product = useProduct(runtime.client, tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'wargoals',
    params: { branchId: track.branch.id, hour, tag } } : null, active && open, validateWargoalsReply, track.source);
  const data = product.data?.hour === hour && product.data.tag === tag && product.data.branchId === track.branch.id ? product.data : null;
  useDiagnostics(onWarnings, active && open ? product.error ? [`War-goal inventory refused (1): ${product.error}`] : data?.warnings ?? EMPTY : EMPTY);
  const targets = (states: readonly number[], liberate: readonly string[] = [], puppets: readonly string[] = []) => <>
    {!!states.length && <p>States: {states.map(id => { const state = data?.stateNames.find(state => state.id === id); return state?.known ? state.name : `Not recorded (#${id})`; }).join(', ')}</p>}
    {!!liberate.length && <p>Liberate: {liberate.join(', ')}</p>}
    {!!puppets.length && <p>Puppet: {puppets.join(', ')}</p>}
  </>;
  return <details open={open} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>War goals{tag ? `: ${tag}` : ''}</summary>
    {!tag ? <p>Choose a country.</p> : product.error ? <p role="alert">{product.error}</p> : !data ? <p role="status">Loading war goals...</p> : <>
      <p>At {dayLabel(hour)}</p>
      {!data.goals.length ? <p>No owned war goals.</p> : data.goals.map((goal, i) => <article key={`${goal.handle}:${i}`}>
        <b>{goal.name} against {goal.recipient}</b>
        <p>Actor: {goal.actor}</p>
        <p>{goal.available ? 'Unused' : 'Owned'}{goal.expires === null ? '; no expiry date' : `; expires ${dayLabel(goal.expires)} (${goal.expires - hour} hours remaining)`}</p>
        {targets(goal.states, goal.liberate, goal.puppets)}
        <Terms value={goal.terms} />
      </article>)}
      <h4>Justifications in progress</h4>
      {!data.justifications.length ? <p>No justifications in progress.</p> : data.justifications.map(j => <article key={j.ordinal}>
        <b>{j.name} against {j.recipient}</b>
        <p>Actor: {j.actor}{j.start === null ? '' : `; started ${dayLabel(j.start)}`}</p>
        <p>Progress: {fixed5(j.points)} / {fixed5(j.total)}; completion {dayLabel(j.completion)}</p>
        <p>{j.payUpfront ? `Paid upfront; daily progress: ${fixed5(j.daily)}` : `Daily political-power cost: ${fixed5(j.daily)}`}{j.paused === true ? '; paused: insufficient political power' : j.paused === null ? '; payment status not recorded' : ''}</p>
        <p>World tension: {fixed5(j.threat)}</p>
        {targets(j.states)}
        <Terms value={j.terms} />
      </article>)}
      {!!data.eligibility.length && <h4>Next actions</h4>}
      {data.eligibility.map(pair => <article key={`${pair.actor}:${pair.recipient}`}>
        <b>{pair.actor} against {pair.recipient}</b>
        <p>Justify war: {pair.justify === null ? 'unresolved' : pair.justify ? 'eligible' : 'blocked'}; {pair.justifyReason}</p>
        <p>Declare war: {pair.declare === null ? 'unresolved' : pair.declare ? 'eligible' : 'blocked'}; {pair.declareReason}</p>
      </article>)}
    </>}
  </details>;
}
