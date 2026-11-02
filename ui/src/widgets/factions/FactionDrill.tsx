import { useCallback, useMemo, useRef } from 'react';
import type { ReactNode } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { usNumber } from '../../charts/numberFormat';
import { bandMaxOf, coveredBy, runState, startUnseen } from '../../charts/timeline/runState';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { MAX_NAME_HOURS } from '../../dashboard/countryFacts';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products';
import type { FactionDetailReply, FactionGoalDefinition, FactionNameRun, HourSpan } from '../../generated/dashboard';
import { countSteps, extracted, fixedWhole, goalGroups, goalName, goalProgress, goalStatus, joinEvents, manifestGoal, manifestMarks, rewardRows, yearStarts } from './factionDrillData';
import { factionLabel, leaderAt, membersAt, tagAt } from './factionsData';
import type { FactionView, MemberView } from './factionsData';

const CHART_HEIGHT = 70;
const noMenu = () => {};

/** One faction at the hour shown, opened from the timeline: who it is, how it grew,
 * what it is working towards and what it has done. Follows playback. */
export default function FactionDrill({ runtime, track, view, names, at, active, gaps, pending, onBack }: {
  runtime: WidgetRuntime; track: BoundTrack; view: FactionView; names: readonly FactionNameRun[]; at: number; active: boolean;
  gaps: readonly HourSpan[]; pending: readonly HourSpan[]; onBack: () => void;
}) {
  const life = view.lifetime;
  // Hours not recorded or still being prepared after it was last seen are not its end.
  const lifeState = runState(life, at, gaps);
  const exists = lifeState.kind === 'held';
  const product = useProduct<FactionDetailReply>(runtime.client, exists ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'faction-details', params: { branchId: track.branch.id, hour: at, faction: view.id } } : null, active);
  // The last reply stays shown while the next hour's loads, so playback never flickers;
  // never one from a later hour (after a rewind), once the current read has failed, or
  // outside the faction's proven existence (dissolved, or not recorded since).
  const held = useRef<FactionDetailReply | null>(null);
  if (product.data?.faction === view.id) held.current = product.data;
  const data = exists && !product.error && held.current?.faction === view.id && held.current.hour <= at ? held.current : null;

  const founded = life[0]?.firstHour ?? null;
  const from = founded == null ? at : Math.min(at, Math.max(track.branch.firstHour, founded));
  // Runs split across unrecorded hours and renames; only another native country is a change of leader.
  const ledBy = view.leaders.filter(run => run.firstHour <= at);
  const leaders = changes(ledBy, run => view.identities.keyAt(run.tag, run.firstHour) ?? JSON.stringify(['tag', run.tag]));
  const leader = leaderAt(view.leaders, at);
  const leaderRun = leaders.at(-1);
  // The former leader as it was when it last led: by the tag it held then, at that hour.
  const formerRun = leaders.length > 1 && leaderRun ? ledBy[ledBy.indexOf(leaderRun) - 1] : null;
  const formerLeader = formerRun?.tag ?? null;
  const formerLeaderHour = formerRun ? formerRun.lastHour : at;
  const nameRuns = changes(names.filter(run => run.firstHour <= at && run.name), run => run.name);
  const formerName = nameRuns.length > 1 ? nameRuns.at(-2)! : null;
  const events = useMemo(() => joinEvents(view, at), [view, at]);
  // A member is named as it was when it last belonged, at or before the hour shown.
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
  const manifest = data ? manifestGoal(data) : null;
  const groups = useMemo(() => data ? goalGroups(data, manifest) : [], [data, manifest]);
  const resources = useMemo(() => extracted(data?.resources ?? null), [data]);

  const goalTip = useCallback((definition: FactionGoalDefinition, subtitle: string, heading: string): TooltipContent => ({
    title: goalName(definition), subtitle, icon: definition.icon ?? undefined, description: definition.description || undefined,
    rows: rewardRows(definition, heading),
  }), []);
  const content = useCallback((key: string): TooltipContent | null => {
    const [kind, id] = [key.slice(0, key.indexOf(':')), key.slice(key.indexOf(':') + 1)];
    if (kind === 'goal') {
      const goal = data?.goals.find(entry => String(entry.ordinal) === id);
      return goal ? goalTip(goal.definition, [goal.definition.categoryName, goalStatus(goal.status)].filter(Boolean).join(' · '), 'On completion') : null;
    }
    if (kind === 'done') {
      const definition = data?.completed[Number(id)];
      return definition ? goalTip(definition, [definition.categoryName, 'Completed'].filter(Boolean).join(' · '), 'Gave') : null;
    }
    if (kind === 'member') {
      const member = view.members.find(entry => entry.key === id);
      const tag = member && memberTag(member);
      const country = member && memberLabel(member);
      // Only the membership at or last before the hour shown, as the timeline says it.
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
      const entry = resources.find(item => String(item.resource.ordinal) === id);
      return entry ? { title: entry.resource.name ?? 'Resource name unavailable', icon: entry.resource.icon ?? undefined,
        subtitle: `${usNumber.format(entry.value)} extracted`, rows: [] } : null;
    }
    return null;
  }, [data, goalTip, view, memberTag, memberLabel, at, leader, track.branch.firstHour, resources]);
  const { rowProps, descriptionId } = useRowTooltips<string>({ owner: `faction-drill:${view.id}`, active, content, revision: `${at}:${data?.hour ?? ''}`, onMenu: noMenu });
  const hover = (key: string) => {
    const shown = content(key);
    return <>{shown && <span id={descriptionId(key)} hidden>{tooltipText(shown)}</span>}</>;
  };

  const name = factionLabel(view, at);
  const leaderLabel = leader ? label(leader) : null;
  const projection = data ? fixedWhole(data.projectionRaw) : null;
  const count = membersAt(view, at);
  const absence = lifeState.kind === 'held' ? null
    : lifeState.kind === 'unknown' && pending.some(span => span.firstHour <= at && at <= span.lastHour) ? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />
      : <WidgetState kind="unavailable" message={lifeState.kind === 'unknown' ? `This faction was not recorded after ${dayLabel(lifeState.lastSeen)}.`
        : lifeState.kind === 'ended' ? `This faction was dissolved ${lifeState.lastSeen == null ? 'on' : 'by'} ${dayLabel(lifeState.at)}.`
          : founded != null && coveredBy(at, founded - 1, gaps) ? 'This faction was not recorded at this time.' : 'This faction does not exist yet at this time.'} />;
  return <div className="faction-drill">
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
        <Stat value={count} label={count === 1 ? 'member' : 'members'} />
        {projection != null && <Stat value={projection} label="power projection" />}
      </div>
    </div>
    {absence ?? <>
      <Section title="Members" note="joined over time">
        <MemberChart view={view} from={from} at={at} events={events} memberLabel={memberLabel} rowProps={rowProps} hover={hover} leaders={leaders} />
      </Section>
      {product.error ? <WidgetState kind="error" failure={product.error} onRetry={product.retry} />
        : !data ? <WidgetState kind="loading" label="Loading faction details…" /> : <>
          <div className="faction-drill-columns">
            <div>
              {manifest && <Section title="Manifest"><Manifest goal={manifest} /></Section>}
              {data.completed.length > 0 && <Section title="Completed" note={usNumber.format(data.completed.length)}>
                <div className="faction-done">{data.completed.map((definition, index) => <span key={index} className="faction-icon-button" {...rowProps(`done:${index}`)}>
                  <GoalIcon definition={definition} />{hover(`done:${index}`)}</span>)}</div>
              </Section>}
            </div>
            <Section title="Goals" note={groups.length ? `${usNumber.format(groups.reduce((sum, group) => sum + group.goals.length, 0))} active` : undefined}>
              {data.goalReason ? <p className="faction-drill-empty">Goals not recorded at this time.</p>
                : !groups.length ? <p className="faction-drill-empty">No goals.</p> : groups.map(group => <div key={group.key} className="faction-term">
                <div className="faction-term-name">{group.name}</div>
                {group.goals.map(goal => <div key={goal.ordinal} className="faction-goal-row" {...rowProps(`goal:${goal.ordinal}`)}>
                  <GoalIcon definition={goal.definition} /><span>{goalName(goal.definition)}</span>{hover(`goal:${goal.ordinal}`)}</div>)}
              </div>)}
            </Section>
          </div>
          {resources.length > 0 && <Section title="Extracted resources">
            <div className="faction-resources">{resources.map(({ resource, value }) => <span key={resource.ordinal} className="faction-resource" {...rowProps(`resource:${resource.ordinal}`)}>
              {resource.icon ? <img src={resource.icon} alt="" /> : <span>{resource.name ?? 'Resource name unavailable'}</span>}<b>{usNumber.format(value)}</b>{hover(`resource:${resource.ordinal}`)}</span>)}</div>
          </Section>}
        </>}
    </>}
  </div>;
}

/** Keeps a marker inside the chart at either end. */
const edge = (percent: number) => percent < 3 ? '0' : percent > 97 ? '-100%' : '-50%';

/** The runs where `key` changes, each from the first hour of its value. */
function changes<T extends { firstHour: number }>(runs: readonly T[], key: (run: T) => unknown): T[] {
  return runs.filter((run, index) => index === 0 || key(run) !== key(runs[index - 1]));
}

function Stat({ value, label }: { value: number | null; label: string }) {
  return <div className="faction-stat"><b>{value == null ? '—' : usNumber.format(value)}</b><span>{label}</span></div>;
}

function Section({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return <section className="faction-drill-section"><h4>{title}{note && <small>{note}</small>}</h4>{children}</section>;
}

function GoalIcon({ definition }: { definition: FactionGoalDefinition }) {
  return definition.icon ? <img className="faction-goal-icon" src={definition.icon} alt="" /> : <span className="faction-goal-icon is-empty" />;
}

function Manifest({ goal }: { goal: FactionDetailReply['goals'][number] }) {
  const progress = goalProgress(goal);
  const { marks, reached } = manifestMarks(goal);
  return <div className="faction-manifest">
    <div className="faction-manifest-name"><b>{goalName(goal.definition)}</b>{reached && <span className="faction-manifest-reached"><Icon name="check" />{reached}</span>}
      {progress != null && <span className="faction-manifest-value">{Math.round(progress)}%</span>}</div>
    {progress != null && <>
      <div className="faction-manifest-bar" role="img" aria-label={`${Math.round(progress)}% progress`}>
        <i style={{ width: `${progress}%` }} />{marks.map(mark => <span key={mark.at} className="faction-manifest-mark" style={{ left: `${mark.at}%` }} />)}
      </div>
      <div className="faction-manifest-scale">
        {marks.map(mark => <span key={mark.at} style={{ left: `${mark.at}%` }} title={mark.name ?? undefined}>{Math.round(mark.at)}%</span>)}
      </div>
    </>}
    {goal.definition.description && <p>{goal.definition.description}</p>}
  </div>;
}

function MemberChart({ view, from, at, events, memberLabel, rowProps, hover, leaders }: {
  view: FactionView; from: number; at: number; events: ReturnType<typeof joinEvents>;
  memberLabel: (member: MemberView) => ReturnType<CountryLabel> | null;
  rowProps: (key: string) => object; hover: (key: string) => ReactNode; leaders: FactionView['leaders'];
}) {
  const span = Math.max(1, at - from);
  const steps = countSteps(view, from, at);
  const most = bandMaxOf([steps]);
  const x = (hour: number) => (hour - from) / span * 1000;
  const y = (value: number) => CHART_HEIGHT - 6 - value / most * (CHART_HEIGHT - 16);
  // Unknown hours between steps break the line and its fill; nothing is drawn across them.
  const runs: (typeof steps)[] = [];
  for (const step of steps) { const run = runs.at(-1); if (run && run.at(-1)!.lastHour + 1 === step.firstHour) run.push(step); else runs.push([step]); }
  const path = (run: typeof steps) => run.map((step, index) => `${index ? 'L' : 'M'}${x(step.firstHour)},${y(step.value)} L${x(step.lastHour + 1)},${y(step.value)}`).join(' ');
  const line = runs.map(path).join(' ');
  const area = runs.map(run => `${path(run)} L${x(run.at(-1)!.lastHour + 1)},${CHART_HEIGHT} L${x(run[0].firstHour)},${CHART_HEIGHT} Z`).join(' ');
  const valueAt = (hour: number) => steps.find(step => step.firstHour <= hour && hour <= step.lastHour)?.value ?? 0;
  const changes = leaders.slice(1).filter(run => run.firstHour >= from);
  // A dark faction colour still reads on a dark panel.
  const ink = `color-mix(in srgb, ${view.color} 65%, var(--text))`;
  return <div className="faction-growth">
    <div className="faction-growth-plot" style={{ height: CHART_HEIGHT }}>
      <svg viewBox={`0 0 1000 ${CHART_HEIGHT}`} preserveAspectRatio="none" aria-hidden="true">
        {area && <path d={area} style={{ fill: ink }} fillOpacity={0.16} />}
        {line && <path d={line} fill="none" style={{ stroke: ink }} strokeWidth={2} vectorEffect="non-scaling-stroke" />}
      </svg>
      {changes.map(run => <span key={run.firstHour} className="faction-growth-crown" style={{ left: `${x(run.firstHour) / 10}%` }}><Icon name="crown" /></span>)}
      {events.map(event => <span key={event.hour} className="faction-growth-join" style={{ left: `${x(event.hour) / 10}%`, top: y(valueAt(event.hour)),
        transform: `translate(${edge(x(event.hour) / 10)}, calc(-100% - 3px))` }}>
        {event.members.map(member => {
          const country = memberLabel(member);
          const current = member.stretches.some(stretch => stretch.firstHour <= at && at <= stretch.lastHour);
          return <span key={member.key} className={`faction-growth-flag${current ? '' : ' is-former'}`} {...rowProps(`member:${member.key}`)}>
            <CountryFlagUrl className="faction-flag" url={country?.flag} detail={country?.flagDetail} reason={country?.flagReason} pending={country?.flagPending} />{hover(`member:${member.key}`)}</span>;
        })}
      </span>)}
    </div>
    <div className="faction-growth-axis">{yearStarts(from, at).map(start => <span key={start.year} style={{ left: `${x(start.hour) / 10}%` }}>{start.year}</span>)}</div>
  </div>;
}
