import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import type { Live, LiveValues } from '../../charts/live';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { usNumber } from '../../charts/numberFormat';
import { bandMaxOf, coveredBy, runState, startUnseen } from '../../charts/timeline/runState';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useFactionDetails } from '../../dashboard/factionChoice';
import { liveTextsOf, projectionKey } from '../../dashboard/factionDetails.ts';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { FactionDetailReply, FactionGoalDefinition, FactionGoalRewardsReply, FactionNameRun, FactionResourceDetail, HourSpan } from '../../generated/dashboard';
import { countSteps, goalGroups, goalName, goalPercent, goalStatus, joinEvents, manifestGoal, manifestMarks, resourceAmount, resourceOrder,
  rewardRows, wholeOf, yearStarts } from './factionDrillData';
import type { HeldGoal, RewardState } from './factionDrillData';
import { factionLabel, leaderAt, membersAt, tagAt } from './factionsData';
import type { FactionView, MemberView } from './factionsData';
import { validateFactionGoalRewards } from './types';
import { productRequest } from '../../dashboard/request.ts';

const CHART_HEIGHT = 70;
const noMenu = () => {};

export default function FactionDrill({ runtime, widget, track, view, names, at, active, gaps, liveHead, onBack }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; view: FactionView; names: readonly FactionNameRun[]; at: number; active: boolean;
  gaps: readonly HourSpan[]; liveHead?: Live<number>; onBack: () => void;
}) {
  const life = view.lifetime;
  const lifeState = runState(life, at, gaps);
  const exists = lifeState.kind === 'held';
  const head = useFactionDetails(runtime, widget, exists ? view.id : null, active);
  const data = exists && !head.error && head.data?.faction === view.id ? head.data : null;
  const live = head.live;
  const dataHour = head.at ?? head.hour ?? at;

  const founded = life[0]?.firstHour ?? null;
  const from = founded == null ? at : Math.min(at, Math.max(track.branch.firstHour, founded));
  const ledBy = view.leaders.filter(run => run.firstHour <= at);
  const leaders = changes(ledBy, run => view.identities.keyAt(run.tag, run.firstHour) ?? JSON.stringify(['tag', run.tag]));
  const leader = leaderAt(view.leaders, at);
  const leaderRun = leaders.at(-1);
  const formerRun = leaders.length > 1 && leaderRun ? ledBy[ledBy.indexOf(leaderRun) - 1] : null;
  const formerLeader = formerRun?.tag ?? null;
  const formerLeaderHour = formerRun ? formerRun.lastHour : at;
  const nameRuns = changes(names.filter(run => run.firstHour <= at && run.name), run => run.name);
  const formerName = nameRuns.length > 1 ? nameRuns.at(-2)! : null;
  const events = useMemo(() => joinEvents(view, at), [view, at]);
  const memberHour = useCallback((member: MemberView) =>
    Math.min(at, member.stretches.filter(stretch => stretch.firstHour <= at).at(-1)?.lastHour ?? at), [at]);
  const memberTag = useCallback((member: MemberView) => tagAt(view, member, memberHour(member)), [view, memberHour]);
  const wanted = useMemo(() => {
    const facts = new Map<string, { hour: number; tag: string }>();
    const add = (tag: string | null, hour: number) => { if (tag) facts.set(JSON.stringify([hour, tag]), { hour, tag }); };
    for (const event of events) for (const member of event.members) add(memberTag(member), memberHour(member));
    add(leader, at);
    add(formerLeader, formerLeaderHour);
    return [...facts.values()];
  }, [events, memberTag, memberHour, leader, formerLeader, formerLeaderHour, at]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active, Math.max(MAX_NAME_HOURS, new Set(wanted.map(item => item.hour)).size));
  const label = labelAt(at);
  const memberLabel = useCallback((member: MemberView) => {
    const tag = memberTag(member);
    return tag ? labelAt(memberHour(member))(tag) : null;
  }, [memberTag, memberHour, labelAt]);
  const manifest = useMemo(() => data ? manifestGoal(data) : null, [data]);
  const groups = useMemo(() => data ? goalGroups(data, manifest) : [], [data, manifest]);
  const resources = data?.resources ?? null;
  const texts = useMemo(() => liveTextsOf(live, (values, key) => {
    if (key === 'projection') { const whole = wholeOf(values.get(projectionKey)); return whole === null ? '—' : usNumber.format(whole); }
    if (key === 'manifest') { const percent = manifest ? goalPercent(values, manifest.goal) : null; return percent === null ? '' : `${Math.round(percent)}%`; }
    const value = resourceAmount(values, { ordinal: Number(key.slice(key.indexOf(':') + 1)) });
    return value === undefined ? 'Not recorded' : usNumber.format(value);
  }), [live, manifest]);

  const [describing, setDescribing] = useState<{ goal: string; hour: number } | null>(null);
  const gate = useGate(runtime, widget, SPEC, active);
  const describedAt = gate.recorded(describing?.hour);
  const described = useProduct<FactionGoalRewardsReply>(runtime.client, describing && describedAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'faction-goal-rewards', params: { branchId: track.branch.id, faction: view.id, goal: describing.goal, hour: describedAt } }) : null,
  active && describing !== null, validateFactionGoalRewards, track.source);
  const rewards = useCallback((definition: FactionGoalDefinition): RewardState | null => {
    if (!definition.token || describing?.goal !== definition.token) return null;
    if (described.error) return { kind: 'failed', reason: described.error.message };
    return described.data?.goal === definition.token ? { kind: 'described', reply: described.data } : { kind: 'loading' };
  }, [describing, described.data, described.error]);
  const goalTip = useCallback((definition: FactionGoalDefinition, subtitle: string, heading: string): TooltipContent => {
    const state = rewards(definition);
    return { title: goalName(definition), subtitle, icon: definition.icon ?? undefined, description: definition.description || undefined,
      rows: state ? rewardRows(state, heading) : [] };
  }, [rewards]);
  const content = useCallback((key: string): TooltipContent | null => {
    const [kind, id] = [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)];
    if (kind === 'goal') {
      const goal = data?.goals.find(entry => String(entry.ordinal) === id);
      const definition = goal && data?.definitions[goal.definition];
      return goal && definition ? goalTip(definition, [definition.categoryName, goalStatus(goal.status)].filter(Boolean).join(' · '), 'On completion') : null;
    }
    if (kind === 'done') {
      const index = data?.completed[Number(id)];
      const definition = index === undefined ? undefined : data?.definitions[index];
      return definition ? goalTip(definition, [definition.categoryName, 'Completed'].filter(Boolean).join(' · '), 'Gave') : null;
    }
    if (kind === 'member') {
      const member = view.members.find(entry => entry.key === id);
      const tag = member && memberTag(member);
      const country = member && memberLabel(member);
      const stretch = member?.stretches.filter(span => span.firstHour <= at).at(-1);
      if (!member || !tag || !country || !stretch) return null;
      const state = runState([stretch], at, view.unknown);
      const since = startUnseen(stretch.firstHour, track.branch.firstHour, view.unknown) ? `before ${dayLabel(stretch.firstHour)}` : dayLabel(stretch.firstHour);
      return { title: country.name, countryFlag: country, rows: [],
        subtitle: state.kind === 'held' ? `${tag === leader ? 'Leader' : 'Member'} since ${since}`
          : state.kind === 'unknown' ? `Member since ${since}, not recorded after ${dayLabel(state.lastSeen)}`
            : state.kind === 'ended' && state.lastSeen != null ? `Member from ${since}, left by ${dayLabel(state.at)}`
              : `Member from ${since} to ${dayLabel(stretch.lastHour)}` };
    }
    if (kind === 'resource') {
      const entry = resources?.find(item => String(item.ordinal) === id);
      if (!entry) return null;
      const value = resourceAmount(live.read(), entry);
      return { title: entry.name ?? 'Resource name unavailable', icon: entry.icon ?? undefined,
        subtitle: value === undefined ? 'Amount not recorded' : `${usNumber.format(value)} extracted`, rows: [] };
    }
    return null;
  }, [data, goalTip, view, memberTag, memberLabel, at, leader, track.branch.firstHour, resources, live]);
  const describedKey = described.error ? `!${describing?.goal ?? ''}` : described.data?.goal ?? '';
  const tips = useRowTooltips<string>({ owner: `faction-drill:${view.id}`, active, content, revision: `${at}:${data ? dataHour : ''}:${describedKey}`, onMenu: noMenu });
  const { rowProps, descriptionId } = tips;
  useEffect(() => {
    const key = tips.hovered;
    const token = key && data ? goalToken(data, key) : null;
    setDescribing(previous => token === null ? null : previous?.goal === token ? previous : { goal: token, hour: dataHour });
  }, [tips.hovered, data]);
  const hover = (key: string) => {
    const shown = content(key);
    return <>{shown && <span id={descriptionId(key)} hidden>{tooltipText(shown)}</span>}</>;
  };

  const pool = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    if (!resources?.length) return undefined;
    const paint = () => orderResources(pool.current, resources, live.read());
    paint();
    return live.subscribe(paint);
  }, [resources, live]);

  const name = factionLabel(view, at);
  const leaderLabel = leader ? label(leader) : null;
  const count = membersAt(view, at);
  const absence = lifeState.kind === 'held' ? null
    : <WidgetState kind="unavailable" message={lifeState.kind === 'unknown' ? `This faction was not recorded after ${dayLabel(lifeState.lastSeen)}.`
      : lifeState.kind === 'ended' ? `This faction was dissolved ${lifeState.lastSeen == null ? 'on' : 'by'} ${dayLabel(lifeState.at)}.`
        : founded != null && coveredBy(at, founded - 1, gaps) ? 'This faction was not recorded at this time.' : 'This faction does not exist yet at this time.'} />;
  return <div className="faction-drill" ref={head.root}>
    <button type="button" className="faction-drill-back" onClick={onBack}><Icon name="chevron-left" />Factions</button>
    <div className="faction-hero">
      {data?.logo ? <img className="faction-logo" src={data.logo} alt="" /> : <span className="faction-logo is-empty" style={{ background: view.color }} />}
      <div className="faction-hero-text">
        <h3>{name}</h3>
        <p>{founded != null && <span>{startUnseen(founded, track.branch.firstHour, gaps) ? `Founded before ${dayLabel(founded)}` : `Founded ${dayLabel(founded)}`}</span>}
          {leaderLabel && <span className="faction-leader">led by <CountryFlagUrl className="faction-flag" url={leaderLabel.flag} detail={leaderLabel.flagDetail} reason={leaderLabel.flagReason} pending={leaderLabel.flagPending} />
            <b>{leaderLabel.name}</b>{formerLeader && leaderRun && <> since {dayLabel(leaderRun.firstHour)}</>}</span>}</p>
        {formerLeader && <p className="faction-former">Previously led by {labelAt(formerLeaderHour)(formerLeader).name}</p>}
        {formerName && <p className="faction-former">Formerly {formerName.name}, renamed {dayLabel(nameRuns.at(-1)!.firstHour)}</p>}
      </div>
      <div className="faction-stats">
        <Stat value={count === null ? '—' : usNumber.format(count)} label={count === 1 ? 'member' : 'members'} />
        {data && <Stat value={<LiveText live={texts} at="projection" />} label="power projection" />}
      </div>
    </div>
    {absence ?? <>
      <Section title="Members" note="joined over time">
        <MemberChart view={view} from={from} at={at} last={track.branch.lastHour} liveHead={liveHead} events={events} memberLabel={memberLabel}
          rowProps={rowProps} hover={hover} leaders={leaders} />
      </Section>
      {head.error ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
        : !data ? <WidgetState kind="loading" label="Loading faction details…" /> : <>
          <div className="faction-drill-columns">
            <div>
              {manifest && <Section title="Manifest"><Manifest held={manifest} live={live} texts={texts} /></Section>}
              {data.completed.length > 0 && <Section title="Completed" note={usNumber.format(data.completed.length)}>
                <div className="faction-done">{data.completed.map((definition, index) => <span key={index} className="faction-icon-button" {...rowProps(`done:${index}`)}>
                  <GoalIcon definition={data.definitions[definition]} />{hover(`done:${index}`)}</span>)}</div>
              </Section>}
            </div>
            <Section title="Goals" note={groups.length ? `${usNumber.format(groups.reduce((sum, group) => sum + group.goals.length, 0))} active` : undefined}>
              {data.goalReason ? <p className="faction-drill-empty">Goals not recorded at this time.</p>
                : !groups.length ? <p className="faction-drill-empty">No goals.</p> : groups.map(group => <div key={group.key} className="faction-term">
                <div className="faction-term-name">{group.name}</div>
                {group.goals.map(({ goal, definition }) => <div key={goal.ordinal} className="faction-goal-row" {...rowProps(`goal:${goal.ordinal}`)}>
                  <GoalIcon definition={definition} /><span>{goalName(definition)}</span>{hover(`goal:${goal.ordinal}`)}</div>)}
              </div>)}
            </Section>
          </div>
          {resources !== null && resources.length > 0 && <Section title="Extracted resources">
            <div className="faction-resources" ref={pool}>{resources.map(resource => <span key={resource.ordinal} className="faction-resource" data-resource={resource.ordinal}
              {...rowProps(`resource:${resource.ordinal}`)}>
              {resource.icon ? <img src={resource.icon} alt="" /> : <span>{resource.name ?? 'Resource name unavailable'}</span>}
              <b><LiveText live={texts} at={`resource:${resource.ordinal}`} /></b>{hover(`resource:${resource.ordinal}`)}</span>)}</div>
          </Section>}
        </>}
    </>}
  </div>;
}

function goalToken(data: FactionDetailReply, key: string): string | null {
  const [kind, id] = [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)];
  const index = kind === 'goal' ? data.goals.find(entry => String(entry.ordinal) === id)?.definition
    : kind === 'done' ? data.completed[Number(id)] : undefined;
  return index === undefined ? null : data.definitions[index]?.token ?? null;
}

function orderResources(root: HTMLElement | null, resources: readonly FactionResourceDetail[], values: LiveValues<number>) {
  if (!root) return;
  const places = resourceOrder(resources, values);
  for (const node of root.querySelectorAll<HTMLElement>('[data-resource]')) {
    const place = String(places.get(Number(node.dataset.resource)) ?? resources.length);
    if (node.style.order !== place) node.style.order = place;
  }
}

const edge = (percent: number) => percent < 3 ? '0' : percent > 97 ? '-100%' : '-50%';

function changes<T extends { firstHour: number }>(runs: readonly T[], key: (run: T) => unknown): T[] {
  return runs.filter((run, index) => index === 0 || key(run) !== key(runs[index - 1]));
}

function Stat({ value, label }: { value: ReactNode; label: string }) {
  return <div className="faction-stat"><b>{value}</b><span>{label}</span></div>;
}

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return <section className="faction-drill-section"><h4>{title}{note && <small>{note}</small>}</h4>{children}</section>;
}

function GoalIcon({ definition }: { definition: FactionGoalDefinition | undefined }) {
  return definition?.icon ? <img className="faction-goal-icon" src={definition.icon} alt="" /> : <span className="faction-goal-icon is-empty" />;
}

function Manifest({ held, live, texts }: { held: HeldGoal; live: Live<LiveValues<number>>; texts: LiveTexts }) {
  const { marks, reached } = manifestMarks(held);
  const value = useRef<HTMLSpanElement | null>(null);
  const bar = useRef<HTMLDivElement | null>(null);
  const fill = useRef<HTMLElement | null>(null);
  useLayoutEffect(() => {
    const paint = () => {
      const percent = goalPercent(live.read(), held.goal);
      const display = percent === null ? 'none' : '';
      for (const node of [value.current, bar.current]) if (node && node.style.display !== display) node.style.display = display;
      if (fill.current && percent !== null) fill.current.style.width = `${percent}%`;
      bar.current?.setAttribute('aria-label', percent === null ? '' : `${Math.round(percent)}% progress`);
    };
    paint();
    return live.subscribe(paint);
  }, [live, held.goal]);
  return <div className="faction-manifest">
    <div className="faction-manifest-name"><b>{goalName(held.definition)}</b>{reached && <span className="faction-manifest-reached"><Icon name="check" />{reached}</span>}
      <span ref={value} className="faction-manifest-value"><LiveText live={texts} at="manifest" /></span></div>
    <div ref={bar}>
      <div className="faction-manifest-bar" role="img">
        <i ref={fill} />{marks.map(mark => <span key={mark.at} className="faction-manifest-mark" style={{ left: `${mark.at}%` }} />)}
      </div>
      <div className="faction-manifest-scale">
        {marks.map(mark => <span key={mark.at} style={{ left: `${mark.at}%` }} title={mark.name ?? undefined}>{Math.round(mark.at)}%</span>)}
      </div>
    </div>
    {held.definition.description && <p>{held.definition.description}</p>}
  </div>;
}

function MemberChart({ view, from, at, last, liveHead, events, memberLabel, rowProps, hover, leaders }: {
  view: FactionView; from: number; at: number; last: number; liveHead?: Live<number>; events: ReturnType<typeof joinEvents>;
  memberLabel: (member: MemberView) => ReturnType<CountryLabel> | null;
  rowProps: (key: string) => object; hover: (key: string) => ReactNode; leaders: FactionView['leaders'];
}) {
  const steps = countSteps(view, from, at, last);
  const most = bandMaxOf([steps]);
  const x = (hour: number) => hour - from;
  const y = (value: number) => CHART_HEIGHT - 6 - value / most * (CHART_HEIGHT - 16);
  const runs: (typeof steps)[] = [];
  for (const step of steps) { const run = runs.at(-1); if (run && run.at(-1)!.lastHour + 1 === step.firstHour) run.push(step); else runs.push([step]); }
  const path = (run: typeof steps) => run.map((step, index) => `${index ? 'L' : 'M'}${x(step.firstHour)},${y(step.value)} L${x(step.lastHour + 1)},${y(step.value)}`).join(' ');
  const line = runs.map(path).join(' ');
  const area = runs.map(run => `${path(run)} L${x(run.at(-1)!.lastHour + 1)},${CHART_HEIGHT} L${x(run[0].firstHour)},${CHART_HEIGHT} Z`).join(' ');
  const valueAt = (hour: number) => steps.find(step => step.firstHour <= hour && hour <= step.lastHour)?.value ?? 0;
  const crowns = leaders.slice(1).filter(run => run.firstHour >= from);
  const ink = `color-mix(in srgb, ${view.color} 65%, var(--text))`;
  const root = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const paint = () => paintGrowth(root.current, Math.max(1, Math.min(last, liveHead ? Math.floor(liveHead.read()) : at) - from));
    paint();
    return liveHead?.subscribe(paint);
  }, [liveHead, at, from, last, steps.length, events.length, crowns.length]);
  const span = Math.max(1, at - from);
  const left = (hour: number) => `${x(hour) / span * 100}%`;
  return <div className="faction-growth" ref={root}>
    <div className="faction-growth-plot" style={{ height: CHART_HEIGHT }}>
      <svg viewBox={`0 0 ${span} ${CHART_HEIGHT}`} preserveAspectRatio="none" aria-hidden="true">
        {area && <path d={area} style={{ fill: ink }} fillOpacity={0.16} />}
        {line && <path d={line} fill="none" style={{ stroke: ink }} strokeWidth={2} vectorEffect="non-scaling-stroke" />}
      </svg>
      {crowns.map(run => <span key={run.firstHour} className="faction-growth-crown" data-x={x(run.firstHour)} style={{ left: left(run.firstHour) }}><Icon name="crown" /></span>)}
      {events.map(event => <span key={event.hour} className="faction-growth-join" data-x={x(event.hour)} data-edge="" style={{ left: left(event.hour), top: y(valueAt(event.hour)),
        transform: `translate(${edge(x(event.hour) / span * 100)}, calc(-100% - 3px))` }}>
        {event.members.map(member => {
          const country = memberLabel(member);
          const current = member.stretches.some(stretch => stretch.firstHour <= at && at <= stretch.lastHour);
          return <span key={member.key} className={`faction-growth-flag${current ? '' : ' is-former'}`} {...rowProps(`member:${member.key}`)}>
            <CountryFlagUrl className="faction-flag" url={country?.flag} detail={country?.flagDetail} reason={country?.flagReason} pending={country?.flagPending} />{hover(`member:${member.key}`)}</span>;
        })}
      </span>)}
    </div>
    <div className="faction-growth-axis">{yearStarts(from, last).map(start => <span key={start.year} data-x={x(start.hour)}
      style={{ left: left(start.hour), display: start.hour > at ? 'none' : undefined }}>{start.year}</span>)}</div>
  </div>;
}

function paintGrowth(root: HTMLElement | null, span: number) {
  if (!root) return;
  const box = `0 0 ${span} ${CHART_HEIGHT}`;
  const svg = root.querySelector('svg');
  if (svg && svg.getAttribute('viewBox') !== box) svg.setAttribute('viewBox', box);
  for (const node of root.querySelectorAll<HTMLElement>('[data-x]')) {
    const offset = Number(node.dataset.x), percent = offset / span * 100;
    const left = `${percent}%`, display = offset > span ? 'none' : '';
    if (node.style.left !== left) node.style.left = left;
    if (node.style.display !== display) node.style.display = display;
    if (node.dataset.edge !== undefined) {
      const transform = `translate(${edge(percent)}, calc(-100% - 3px))`;
      if (node.style.transform !== transform) node.style.transform = transform;
    }
  }
}

