import { useMemo } from 'react';
import type { ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import type { WidgetProps } from '../../dashboard/definitions';
import { selectedTags } from '../../dashboard/countrySelection';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useFollowedProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { WargoalStanding, WargoalsReply } from '../../generated/dashboard';
import { fixed5, validateWargoalsReply } from './warGoalsData';
import './warGoals.css';

const EMPTY: string[] = [];
const STANDING: Record<WargoalStanding, string> = { atWar: 'At war', sameFaction: 'Same faction' };

export default function WarGoalsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window ? Math.floor(window.lastHour) : null;
  const tag = track ? selectedTags(widget.countries, track)[0] ?? null : null;
  // The last hour's goals stay shown, labelled with their own hour, while playback's newest is read.
  const product = useFollowedProduct<WargoalsReply>(runtime.client, track && tag && hour != null ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'wargoals', params: { branchId: track.branch.id, hour, tag } } : null, ['hour'], active, validateWargoalsReply, track?.source);
  const data = product.data?.tag === tag && product.data.branchId === track?.branch.id ? product.data : null;
  useDiagnostics(onDiagnostics, active ? product.error ? [`War-goal inventory refused (1): ${product.error}`] : data?.warnings ?? EMPTY : EMPTY);
  const wanted = useMemo(() => data ? [...new Set([data.tag, ...data.goals.flatMap(goal => [goal.actor, goal.recipient, ...goal.liberate, ...goal.puppets]),
    ...data.justifications.map(entry => entry.recipient)])].map(country => ({ hour: data.hour, tag: country })) : [], [data]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && !!data);
  const standings = useMemo(() => new Map(data?.standings.map(pair => [JSON.stringify([pair.actor, pair.recipient]), pair.standing]) ?? []), [data]);

  if (!track) return <WidgetState kind="empty" message="Load a campaign to see war goals." />;
  if (hour == null) return <WidgetState kind="unavailable" message="Choose a recorded time." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country to see its war goals." />;
  if (product.error && !data) return <WidgetState kind="error" detail={product.error} onRetry={product.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading war goals…" />;

  const label = labelAt(data.hour);
  const country = (code: string) => {
    const { name, flag, flagReason, flagPending } = label(code);
    return <span className="war-goal-country"><CountryFlagUrl className="war-goal-flag" url={flag} reason={flagReason} pending={flagPending} /><b>{name}</b></span>;
  };
  const stateName = (id: number) => {
    const state = data.stateNames.find(entry => entry.id === id);
    return state?.known ? state.name : `State not recorded (#${id})`;
  };
  const states = (ids: readonly number[]) => ids.map((id, index) => <span key={`state:${index}`} className="war-goal-chip">{stateName(id)}</span>);
  const atWar = data.goals.filter(goal => standings.get(JSON.stringify([goal.actor, goal.recipient])) === 'atWar').length;
  return <div className="war-goals">
    <div className="war-goals-summary">
      <span><b>{usNumber.format(data.goals.length)}</b> {data.goals.length === 1 ? 'war goal' : 'war goals'}</span>
      {atWar > 0 && <span><b>{usNumber.format(atWar)}</b> against countries at war</span>}
      <span><b>{usNumber.format(data.justifications.length)}</b> being justified</span>
    </div>
    <Section title="Owned goals" count={data.goals.length} empty="No owned war goals.">
      {data.goals.map((goal, index) => {
        const standing = standings.get(JSON.stringify([goal.actor, goal.recipient])) ?? null;
        const remaining = goal.expires == null ? null : goal.expires - data.hour;
        const targets = goal.states.length || goal.liberate.length || goal.puppets.length;
        return <div key={`${goal.handle}:${index}`} className="war-goal">
          <div className="war-goal-head">
            {country(goal.recipient)}<span className="war-goal-kind">{goal.name}</span><span className="war-goal-spacer" />
            {goal.available && <span className="war-goal-pill is-unused">Unused</span>}
            {standing && <span className={`war-goal-pill is-${standing}`}>{standing === 'atWar' && <Icon name="swords" />}{STANDING[standing]}</span>}
          </div>
          {(targets || remaining != null || goal.actor !== data.tag) ? <div className="war-goal-sub">
            {states(goal.states)}
            {goal.liberate.map(code => <span key={`liberate:${code}`} className="war-goal-chip">Liberate {label(code).name}</span>)}
            {goal.puppets.map(code => <span key={`puppet:${code}`} className="war-goal-chip">Puppet {label(code).name}</span>)}
            {remaining != null && <span className="war-goal-when">{remaining > 0
              ? `Expires ${dayLabel(goal.expires!)} · ${usNumber.format(Math.ceil(remaining / 24))} ${Math.ceil(remaining / 24) === 1 ? 'day' : 'days'}`
              : `Expiry date passed ${dayLabel(goal.expires!)}`}</span>}
            {goal.actor !== data.tag && <span className="war-goal-when">Held by {label(goal.actor).name}</span>}
          </div> : null}
        </div>;
      })}
    </Section>
    <Section title="Being justified" count={data.justifications.length} empty="No justifications in progress.">
      {data.justifications.map(entry => {
        const total = BigInt(entry.total), points = BigInt(entry.points);
        const share = total > 0n ? Number(((points < 0n ? 0n : points > total ? total : points) * 1000n) / total) / 10 : null;
        return <div key={entry.ordinal} className="war-goal">
          <div className="war-goal-head">
            {country(entry.recipient)}<span className="war-goal-kind">{entry.name}</span><span className="war-goal-spacer" />
            {entry.paused === true && <span className="war-goal-pill is-paused">Paused: not enough political power</span>}
            {entry.paused === null && <span className="war-goal-when">Payment status not recorded</span>}
          </div>
          {share != null && <div className="war-goal-bar" role="img" aria-label={`${share}% justified`}><i style={{ width: `${share}%` }} /></div>}
          <div className="war-goal-sub">
            <span className="war-goal-when">{share != null && <><b>{share}%</b> · </>}{fixed5(entry.points)} / {fixed5(entry.total)}</span>
            <span className="war-goal-when">completes {dayLabel(entry.completion)}</span>
            <span className="war-goal-when">{entry.payUpfront ? `paid upfront · ${fixed5(entry.daily)} progress a day` : `${fixed5(entry.daily)} political power a day`}</span>
          </div>
          {entry.states.length > 0 && <div className="war-goal-sub">{states(entry.states)}</div>}
        </div>;
      })}
    </Section>
  </div>;
}

function Section({ title, count, empty, children }: { title: string; count: number; empty: string; children: ReactNode }) {
  return <section className="war-goals-section">
    <h3>{title} <small>{usNumber.format(count)}</small></h3>
    {count ? children : <p className="war-goals-empty">{empty}</p>}
  </section>;
}
