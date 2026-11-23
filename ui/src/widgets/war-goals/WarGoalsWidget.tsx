import { answersKey, countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useLayoutEffect, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import type { WidgetProps } from '../../dashboard/definitions';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { resolveWindow } from '../../dashboard/model';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { expiryText, fixed5, progressText, validateWargoalsReply, wargoalStructure } from './warGoalsData';
import type { WargoalsReply } from '../../generated/dashboard';
import LiveText, { liveTexts } from '../../charts/LiveText';
import { liveKey } from '../../charts/live';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import './warGoals.css';
import type { WargoalStanding } from '../../generated/dashboard';

const EMPTY: string[] = [];
const STANDING: Record<WargoalStanding, string> = { atWar: 'At war', sameFaction: 'Same faction' };

export default function WarGoalsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const clock = runtime.clock;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, clock.getHour());
  const hour = window ? Math.floor(window.lastHour) : null;
  const tag = track ? selectedCountryKeys(widget.countries, track)[0] ?? null : null;
  const root = useRef<HTMLDivElement | null>(null);
  const paintCompletions = useCallback((reply: WargoalsReply) => {
    for (const entry of reply.justifications) {
      const node = root.current?.querySelector<HTMLElement>(`[data-completion="${entry.id}"]`);
      const text = `completes ${dayLabel(entry.completion)}`;
      if (node && node.textContent !== text) node.textContent = text;
    }
  }, []);
  const product = useHeadProduct(runtime, widget, 'wargoals', tag ? { country: countryFromKey(tag) } : null,
    { active, validate: validateWargoalsReply, structure: wargoalStructure, paint: paintCompletions });
  const data = answersKey(product.data?.asked, tag) && product.data?.branchId === track?.branch.id ? product.data : null;
  const at = product.at ?? hour ?? 0;
  useShownEachFrame(clock, () => data ? product.shownAt() : null, product.live);
  const live = product.live;
  useLayoutEffect(() => {
    const paint = () => {
      const values = live.read();
      for (const node of root.current?.querySelectorAll<HTMLElement>('[data-points]') ?? []) {
        const key = liveKey(node.dataset.points!, 'points');
        const { share, shareText, text } = progressText(values.get(key), node.dataset.total!, values.missing(key));
        const [bold, rest] = [node.firstElementChild, node.lastElementChild];
        if (bold && bold.textContent !== shareText) bold.textContent = shareText;
        if (rest && rest.textContent !== text) rest.textContent = text;
        const bar = root.current?.querySelector<HTMLElement>(`[data-bar="${node.dataset.points}"]`);
        if (bar) {
          bar.hidden = share === null;
          const width = `${share ?? 0}%`;
          if (bar.firstElementChild instanceof HTMLElement && bar.firstElementChild.style.width !== width) bar.firstElementChild.style.width = width;
          bar.setAttribute('aria-label', share === null ? text : `${shareText} justified`);
        }
      }
    };
    paint();
    return live.subscribe(paint);
  });
  const shownAt = useRef(product.shownAt);
  shownAt.current = product.shownAt;
  const expiries = useMemo(() => liveTexts(clock, () => shownAt.current() ?? at,
    now => new Map((data?.goals ?? []).flatMap(goal => goal.expires == null ? [] : [[goal.handle, expiryText(goal.expires, now)] as const]))),
  [clock, at, data]);
  useDiagnostics(onDiagnostics, active ? product.error ? [`War-goal inventory refused (1): ${product.error.message}`] : data?.warnings ?? EMPTY : EMPTY);
  const wanted = useMemo(() => data ? [...new Set([data.tag, ...data.goals.flatMap(goal => [goal.actor, goal.recipient, ...goal.liberate, ...goal.puppets]),
    ...data.justifications.map(entry => entry.recipient)])].map(country => ({ hour: at, tag: country })) : [], [data, at]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && !!data);
  const standings = useMemo(() => new Map(data?.standings.map(pair => [JSON.stringify([pair.actor, pair.recipient]), pair.standing]) ?? []), [data]);

  if (!track) return <WidgetState kind="empty" message="Load a campaign to see war goals." />;
  if (hour == null) return <WidgetState kind="unavailable" message="Choose a recorded time." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country to see its war goals." />;
  if (product.error && !data) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading war goals…" />;

  const label = labelAt(at);
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
  return <div ref={element => { root.current = element; product.root(element); }} className="war-goals">
    <div className="war-goals-summary">
      <span><b>{usNumber.format(data.goals.length)}</b> {data.goals.length === 1 ? 'war goal' : 'war goals'}</span>
      {atWar > 0 && <span><b>{usNumber.format(atWar)}</b> against countries at war</span>}
      <span><b>{usNumber.format(data.justifications.length)}</b> being justified</span>
      { }
      {product.read !== 'followed' && product.at !== null && product.at !== hour && <span className="war-goals-asof">As of {dayLabel(product.at)}; {dayLabel(hour)} still loading</span>}
    </div>
    <Section title="Owned goals" count={data.goals.length} empty="No owned war goals.">
      {data.goals.map((goal, index) => {
        const standing = standings.get(JSON.stringify([goal.actor, goal.recipient])) ?? null;
        const targets = goal.states.length || goal.liberate.length || goal.puppets.length;
        return <div key={`${goal.handle}:${index}`} className="war-goal">
          <div className="war-goal-head">
            {country(goal.recipient)}<span className="war-goal-kind">{goal.name}</span><span className="war-goal-spacer" />
            {goal.available && <span className="war-goal-pill is-unused">Unused</span>}
            {standing && <span className={`war-goal-pill is-${standing}`}>{standing === 'atWar' && <Icon name="swords" />}{STANDING[standing]}</span>}
          </div>
          {(targets || goal.expires != null || goal.actor !== data.tag) ? <div className="war-goal-sub">
            {states(goal.states)}
            {goal.liberate.map(code => <span key={`liberate:${code}`} className="war-goal-chip">Liberate {label(code).name}</span>)}
            {goal.puppets.map(code => <span key={`puppet:${code}`} className="war-goal-chip">Puppet {label(code).name}</span>)}
            {goal.expires != null && <span className="war-goal-when"><LiveText live={expiries} at={goal.handle} /></span>}
            {goal.actor !== data.tag && <span className="war-goal-when">Held by {label(goal.actor).name}</span>}
          </div> : null}
        </div>;
      })}
    </Section>
    <Section title="Being justified" count={data.justifications.length} empty="No justifications in progress.">
      {data.justifications.map(entry => {
        return <div key={entry.id} className="war-goal">
          <div className="war-goal-head">
            {country(entry.recipient)}<span className="war-goal-kind">{entry.name}</span><span className="war-goal-spacer" />
            {entry.paused === true && <span className="war-goal-pill is-paused">Paused: not enough political power</span>}
            {entry.paused === null && <span className="war-goal-when">Payment status not recorded</span>}
          </div>
          <div className="war-goal-bar" role="img" data-bar={entry.id} hidden><i /></div>
          <div className="war-goal-sub">
            <span className="war-goal-when" data-points={entry.id} data-total={entry.total}><b /><span /></span>
            <span className="war-goal-when" data-completion={entry.id}>completes {dayLabel(entry.completion)}</span>
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
