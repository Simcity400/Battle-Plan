import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import { dayLabel } from '../../charts/draw';
import { CountryFlag } from '../../components/CountryFlag';
import type { FamilyViewCountryChipProps, FamilyViewProps, FamilyViewRenderer, FamilyViewSubjectMenuProps, ResolvedViewOptions } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveWidgetCampaign } from '../../dashboard/campaigns';
import { setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products.ts';
import { failureText } from '../../dashboard/queryError.ts';
import { entryAt } from '../../dashboard/reads.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import { watchBoundary } from '../../dashboard/widgetTime';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry';
import { countryTagKey, useIdentityAtHead, useIdentityScope, type CountryIdentityResult } from '../../families/identity/react.ts';
import { observationAt, validateWarsReply } from '../wars/warsData.ts';
import ContributorWindow, { type OpenedContributor } from './ContributorWindow.tsx';
import { DEFAULT_MEASURE, MEASURE_LIST } from './definition.ts';
import { WAR_NOT_STARTED, WAR_UNAVAILABLE, chosenWar, createWarChipTracker, readHourFor, warChipLeaderHour } from './warContributionData.ts';
import { attackersFirst, savedWar } from './warContributionPlan.ts';

const storedWar = (options: ResolvedViewOptions) => savedWar({ war: options.war });
const NO_SUBSCRIBE = () => () => {};
import { useWarChoices, useWarMenu, useWars } from './useWarChoices';
import './warContribution.css';

function WarContributionView(props: FamilyViewProps) {
  const { setOption, manifestDeclaration, widget, runtime, active, options } = props;
  const { track, within, country } = useGate(runtime, widget, undefined, active, { country: { stored: options.country } });
  const viewpoint = country?.tag ?? null;
  const wars = useProduct(runtime.client, track && viewpoint ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'wars', params: { branchId: track.branch.id, viewpoint } }) : null, active, validateWarsReply, track?.source).data;
  const saved = storedWar(options);
  const scope = JSON.stringify([track?.id, track?.source.id, track?.source.revision, track?.branch.id, country?.chosen, options.country, saved]);
  const anchor = useRef<HTMLDivElement>(null);
  const [opened, setOpened] = useState<(OpenedContributor & { scope: string }) | null>(null);
  const shown = active && opened?.scope === scope ? opened : null;
  useEffect(() => { setOpened(null); }, [scope, active]);
  const onActivate = (action: InstalledAction) => {
    const pick = (action.meta as { readonly pick?: string } | undefined)?.pick;
    if (pick) { setOption?.('measure', pick); return; }
    if (!track || !wars || !viewpoint || action.target.kind !== 'row') return;
    const hit = /^(?:(viewpoint|opposing):([^:]+)|total:(viewpoint|opposing))$/.exec(action.target.key);
    const war = hit && chosenWar(wars, saved, action.hour);
    if (!hit || !war) return;
    const at = readHourFor(war, action.hour);
    const observation = observationAt(war, at);
    const side = (hit[1] ?? hit[3]) === 'viewpoint' ? 0 : 1;
    const tag = hit[2] ?? null;
    if (!observation || tag !== null && !(side === 0 ? observation.viewpointSide : observation.opposingSide).includes(tag)) return;
    const tail = entryAt(wars.unobserved, war.lastHour + 1);
    const through = within(war.endedBy && tail?.firstHour === war.lastHour + 1 ? Math.max(at, Math.min(action.hour, tail.lastHour)) : at);
    if (through === null) return;
    const measure = MEASURE_LIST.find(item => item.key === options.measure)?.key ?? DEFAULT_MEASURE;
    const { name } = observation;
    const request = productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'war-contribution-history',
      params: { branchId: track.branch.id, viewpoint, warId: war.id, throughHour: through } });
    setOpened({ scope, request, warName: name.known ? name.text : name.reason ?? 'War name not recorded', at, side, tag, measure });
  };
  if (!manifestDeclaration) return null;
  return <div ref={anchor} className="fh-detail-anchor">
    <FamilyWidget {...props} family={manifestDeclaration.family} declaration={manifestDeclaration} onActivate={onActivate} />
    {shown && track && <ContributorWindow key={`${scope}:${JSON.stringify(shown.request.params)}`} opened={shown}
      track={track} runtime={runtime} anchor={anchor} onClose={() => setOpened(null)} />}
  </div>;
}

function useChosenWar(props: Pick<FamilyViewProps, 'widget' | 'runtime' | 'active' | 'options'>) {
  const { widget, runtime, active, options } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const wars = useWars(widget, runtime, active);
  const saved = storedWar(options);
  const shownHour = useCallback((clockHour: number) => (track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null), [track, widget, runtime.tracks]);
  const offset = track?.axisOffset ?? 0;
  const tracker = useMemo(
    () => createWarChipTracker(wars, saved, shownHour, offset),
    [wars, saved, shownHour, offset],
  );
  const follow = active && !!wars && !!track && widget.time.mode === 'follow';
  const subscribe = useCallback(
    (changed: () => void) => (follow ? watchBoundary(runtime.clock, tracker.span, changed) : NO_SUBSCRIBE()),
    [follow, runtime.clock, tracker],
  );
  const read = useCallback(() => tracker.read(runtime.clock.getHour()), [tracker, runtime.clock]);
  const chip = useSyncExternalStore(subscribe, read, read);
  return { track, shownHour, ...chip };
}

function WarChip(props: FamilyViewCountryChipProps) {
  const { track, shownHour, observation, prestart, unavailable } = useChosenWar(props);
  const [firstTag, secondTag] = attackersFirst(observation, observation?.viewpointSide[0], observation?.opposingSide[0]);
  const hasPair = Boolean(firstTag && secondTag);
  const leaderKeys = useMemo(
    () => (firstTag && secondTag ? [countryTagKey(firstTag), countryTagKey(secondTag)] : []),
    [firstTag, secondTag],
  );
  const scope = useIdentityScope(props.runtime.client, track?.source, track?.branch.id ?? null, props.active && hasPair);
  const branchBounds = useMemo(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] as const : undefined),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const leaderHour = useCallback(
    (clockHour: number) => warChipLeaderHour(observation, clockHour, shownHour),
    [observation, shownHour],
  );
  const identity = useIdentityAtHead(props.runtime.client, track?.source, scope, props.runtime.clock, leaderKeys, leaderHour, {
    active: props.active && hasPair,
    follow: props.widget.time.mode === 'follow',
    needFlags: true,
    flagSize: 'small',
    branchBounds,
  });
  if (!firstTag || !secondTag) {
    const label = prestart ? WAR_NOT_STARTED : unavailable ? WAR_UNAVAILABLE : 'Choose a war';
    return <span className="dash-widget-country war-chip"><span>{label}</span></span>;
  }
  const readLeader = (index: number, tag: string) => {
    const result = identity.results[index] as CountryIdentityResult | undefined;
    const country = result?.kind === 'known' ? result : null;
    const name = country && country.name.kind === 'known' ? country.name.value : tag;
    const choice = country && country.flagChoice.kind === 'known' ? country.flagChoice.value : null;
    const flag = choice ? identity.flagFor(choice) : undefined;
    const pending = !result || result.kind === 'pending'
      || (country !== null && (country.flagChoice.kind === 'pending'
        || (choice !== null && identity.flagArtFor(choice).status === 'pending')));
    return { name, flag, pending };
  };
  const first = readLeader(0, firstTag);
  const second = readLeader(1, secondTag);
  const name = `${first.name} – ${second.name}`;
  return <span className="dash-widget-country war-chip" data-tooltip={props.compact ? name : undefined}>
    <CountryFlag image={first.flag?.image} tag={firstTag} name={first.name} pending={first.pending} decorative />
    <span className="war-chip-dash">–</span>
    <CountryFlag image={second.flag?.image} tag={secondTag} name={second.name} pending={second.pending} decorative />
    <span>{name}</span>
  </span>;
}

function WarMenu({ widget, runtime, options, setOption }: FamilyViewSubjectMenuProps) {
  const saved = storedWar(options);
  const { reply, choices } = useWarMenu(resolveWidgetCampaign({ ...widget, options: {} }, { tracks: runtime.campaignTracks }, runtime.defaultRoleId), runtime, saved);
  const wars = reply.data;
  const pending = !wars ? (reply.error ? failureText(reply.error, 'The wars were not read') : 'Loading wars…') : wars.available ? null : 'Wars are not in this recording.';
  const choose = (id: string | null) => setOption('war', id === null ? '' : setToken(id));
  return <div className="war-menu" role="group" aria-label="War">
    <div className="war-menu-heading">War</div>
    <button type="button" className={`war-menu-row${saved === null ? ' is-chosen' : ''}`} onClick={() => choose(null)}>The country’s main war at the date</button>
    {pending && <div className="war-menu-pending" role="status">{pending}</div>}
    {choices.map(war => <button key={war.id} type="button" className={`war-menu-row${saved === war.id ? ' is-chosen' : ''}`} onClick={() => choose(war.id)}>
      <span>{war.name}</span>{war.firstHour !== null && <small>{`From ${dayLabel(war.firstHour)}`}</small>}
    </button>)}
  </div>;
}

export default { render: WarContributionView, countryChip: WarChip, subjectMenu: WarMenu, useOptionChoices: useWarChoices } satisfies FamilyViewRenderer;
