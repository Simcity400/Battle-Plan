import { useMemo } from 'react';
import { CountryFlagUrl } from '../../components/CountryFlag';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useFactionDetails } from '../../dashboard/factionChoice';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { urlToArt } from '../../families/art/adapters.ts';
import { useDomArt } from '../../families/art/react.ts';
import { countryTagKey, useIdentity, useIdentityScope } from '../../families/identity/react.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { FactionDetailReply, FactionGoalDefinition } from '../../generated/dashboard';
import { leaderAt, membersAt, tagAt, type FactionView } from './factionsData';
import './factionDetails.css';

interface Props {
  readonly runtime: WidgetRuntime;
  readonly widget: WidgetSpec;
  readonly track: BoundTrack;
  readonly view: FactionView;
  readonly hour: number;
  readonly active: boolean;
}

function Country({ runtime, track, tag, hour }: { readonly runtime: WidgetRuntime; readonly track: BoundTrack; readonly tag: string; readonly hour: number }) {
  const scope = useIdentityScope(runtime.client, track.source, track.branch.id);
  const key = useMemo(() => countryTagKey(tag), [tag]);
  const bounds = useMemo(() => [track.branch.firstHour, track.branch.lastHour] as const, [track.branch.firstHour, track.branch.lastHour]);
  const { result, flagArt } = useIdentity(runtime.client, track.source, scope, key, hour, { needFlag: true, flagSize: 'panel', branchBounds: bounds });
  const name = result && result.kind === 'known' && 'name' in result && result.name.kind === 'known' ? result.name.value : tag;
  const url = flagArt.status === 'ready' ? flagArt.url : null;
  const reason = flagArt.status === 'confirmedMissing' || flagArt.status === 'refused' ? flagArt.reason : undefined;
  return <span className="faction-details-country">
    <CountryFlagUrl className="faction-details-flag" url={url ?? undefined} pending={flagArt.status === 'pending'} reason={reason} tag={tag} name={name} />
    <span>{name}</span>
  </span>;
}

function goalName(definition: FactionGoalDefinition | undefined): string {
  return definition?.name ?? definition?.token ?? 'Goal name not recorded';
}

function Logo({ url }: { readonly url: string | null }) {
  const art = useDomArt(useMemo(() => urlToArt(url ?? undefined, { reason: url ? undefined : 'Faction logo not drawn' }), [url]));
  return art.status === 'ready' && art.bitmap ? <img className="faction-details-logo" src={art.bitmap.src} alt="" /> : <span className="faction-details-logo is-empty" />;
}

function Goals({ details }: { readonly details: FactionDetailReply }) {
  if (details.goalReason) return <p className="faction-details-missing">Goals not recorded: {details.goalReason}</p>;
  const byTerm = new Map<string, { readonly name: string; readonly goals: string[] }>();
  for (const goal of details.goals) {
    if ((goal.status !== 0 && goal.status !== 3) || goal.definition === details.manifest) continue;
    const definition = details.definitions[goal.definition];
    const term = definition?.category ?? '';
    const entry = byTerm.get(term) ?? { name: definition?.categoryName ?? 'Goals', goals: [] };
    entry.goals.push(goalName(definition));
    byTerm.set(term, entry);
  }
  const manifest = details.manifest === null ? null : details.definitions[details.manifest];
  return <>
    {manifest ? <p className="faction-details-row"><span>Manifest</span><b>{goalName(manifest)}</b></p> : null}
    {[...byTerm.values()].map(term => <div key={term.name} className="faction-details-term">
      <h5>{term.name}</h5>
      <ul>{term.goals.map((name, index) => <li key={index}>{name}</li>)}</ul>
    </div>)}
    <p className="faction-details-row"><span>Completed</span><b>{details.completed.length}</b></p>
  </>;
}

export default function FactionDetails({ runtime, widget, track, view, hour, active }: Props) {
  const head = useFactionDetails(runtime, widget, view.id, active);
  const details = head.data?.faction === view.id ? head.data : null;
  const leader = leaderAt(view.leaders, hour);
  const count = membersAt(view, hour);
  const members = view.members.flatMap(member => {
    const run = member.runs.find(item => item.firstHour <= hour && hour <= item.lastHour);
    const tag = run ? tagAt(view, member, hour) : null;
    return run && tag ? [{ key: member.key, tag, since: member.runs[0]!.firstHour }] : [];
  });
  return <div className="faction-details">
    <header className="faction-details-head">
      <Logo url={details?.logo ?? null} />
      <div>
        {leader ? <p className="faction-details-row"><span>Leader</span><Country runtime={runtime} track={track} tag={leader} hour={hour} /></p> : null}
      </div>
    </header>
    <section>
      <h4>Members</h4>
      {count === null ? <p className="faction-details-missing">Members not recorded at this hour</p> : null}
      <ul className="faction-details-members">
        {members.map(member => <li key={member.key}>
          <Country runtime={runtime} track={track} tag={member.tag} hour={hour} />
          <small>{member.since <= track.branch.firstHour ? `Member before ${formatDateOnly(member.since)}` : `Joined ${formatDateOnly(member.since)}`}</small>
        </li>)}
      </ul>
    </section>
    <section>
      <h4>Goals</h4>
      {head.error ? <p className="faction-details-missing">Faction details could not be loaded: {head.error.message}</p>
        : details ? <Goals details={details} /> : <p className="faction-details-missing">Loading</p>}
    </section>
    {details ? <section>
      <h4>Resources</h4>
      {details.resources === null ? <p className="faction-details-missing">Resources not recorded at this hour</p>
        : <ul className="faction-details-resources">{details.resources.map(resource => <li key={resource.ordinal}>{resource.name ?? resource.token ?? resource.reason ?? 'Resource name not recorded'}</li>)}</ul>}
    </section> : null}
  </div>;
}
