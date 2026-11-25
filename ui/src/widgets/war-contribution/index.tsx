import { useCallback, useMemo } from 'react';
import { dayLabel } from '../../charts/draw';
import { CountryFlag } from '../../components/CountryFlag';
import type { FamilyViewCountryChipProps, FamilyViewProps, FamilyViewRenderer, FamilyViewSubjectMenuProps, ResolvedViewOptions } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry';
import { useIdentity, useIdentityScope } from '../../families/identity/react.ts';
import { countryTagKey } from '../../families/identity/types.ts';
import { useHeadHour } from '../army-size/headHour';
import { labelObservationAt } from '../wars/warsData';
import { MEASURE_VIEW } from './measureProjection.ts';
import { chosenWar } from './warContributionData.ts';
import { savedWar } from './warContributionPlan.ts';

const storedWar = (options: ResolvedViewOptions) => savedWar({ war: options.war });
import { useWarChoices, useWars } from './useWarChoices';
import './warContribution.css';

function WarContributionView(props: FamilyViewProps) {
  const { setOption, manifestDeclaration } = props;
  const onActivate = useCallback((action: InstalledAction) => {
    const pick = (action.meta as { readonly pick?: string } | undefined)?.pick;
    if (pick) setOption?.('measure', pick);
  }, [setOption]);
  if (!manifestDeclaration) return null;
  return <FamilyWidget {...props} family={manifestDeclaration.family} declaration={manifestDeclaration} {...(props.view === MEASURE_VIEW ? { onActivate } : {})} />;
}

function useChosenWar(props: Pick<FamilyViewProps, 'widget' | 'runtime' | 'active' | 'options'>) {
  const { widget, runtime, active, options } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const wars = useWars(widget, runtime, active);
  const saved = storedWar(options);
  const shownHour = useCallback((clockHour: number) => (track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null), [track, widget, runtime.tracks]);
  const hour = useHeadHour(runtime.clock, active && widget.time.mode === 'follow', clockHour => {
    const at = shownHour(clockHour);
    return at === null || !wars ? '' : chosenWar(wars, saved, at)?.id ?? 'none';
  });
  const at = shownHour(hour);
  const war = at !== null && wars ? chosenWar(wars, saved, at) : null;
  const observation = war && at !== null ? labelObservationAt(war, Math.min(at, war.lastHour)) : null;
  return { track, war, observation, at };
}

function useLeader(props: Pick<FamilyViewProps, 'runtime' | 'active'>, track: ReturnType<typeof useChosenWar>['track'], tag: string | undefined, hour: number | null) {
  const scope = useIdentityScope(props.runtime.client, track?.source, track?.branch.id ?? null, props.active && !!tag);
  const key = useMemo(() => (tag ? countryTagKey(tag) : null), [tag]);
  const { result, flag, flagArt } = useIdentity(props.runtime.client, track?.source, scope, key, hour, { active: props.active && !!tag, needFlag: true, flagSize: 'small' });
  const name = result?.kind === 'known' && result.name.kind === 'known' ? result.name.value : tag ?? '';
  return { name, flag, pending: flagArt.status === 'pending' };
}

function WarChip(props: FamilyViewCountryChipProps) {
  const { track, observation, at } = useChosenWar(props);
  const shown = observation && at !== null ? Math.min(at, observation.lastHour) : null;
  const first = useLeader(props, track, observation?.viewpointSide[0], shown);
  const second = useLeader(props, track, observation?.opposingSide[0], shown);
  if (!observation || !observation.viewpointSide[0] || !observation.opposingSide[0]) return <span className="dash-widget-country war-chip"><span>Choose a war</span></span>;
  const name = `${first.name} – ${second.name}`;
  return <span className="dash-widget-country war-chip" data-tooltip={props.compact ? name : undefined}>
    <CountryFlag image={first.flag?.image} tag={observation.viewpointSide[0]} name={first.name} pending={first.pending} decorative />
    <span className="war-chip-dash">–</span>
    <CountryFlag image={second.flag?.image} tag={observation.opposingSide[0]} name={second.name} pending={second.pending} decorative />
    <span>{name}</span>
  </span>;
}

function WarMenu({ widget, runtime, options, setOption }: FamilyViewSubjectMenuProps) {
  const wars = useWars(widget, runtime, true);
  const saved = storedWar(options);
  const choose = (id: string | null) => setOption('war', id === null ? '' : setToken(id));
  return <div className="war-menu" role="group" aria-label="War">
    <div className="war-menu-heading">War</div>
    <button type="button" className={`war-menu-row${saved === null ? ' is-chosen' : ''}`} onClick={() => choose(null)}>The country’s main war at the date</button>
    {(wars?.wars ?? []).map(war => {
      const observation = war.observations.at(-1) ?? null;
      const name = observation?.name.known ? observation.name.text : 'War name not recorded';
      return <button key={war.id} type="button" className={`war-menu-row${saved === war.id ? ' is-chosen' : ''}`} onClick={() => choose(war.id)}>
        <span>{name}</span><small>{`From ${dayLabel(war.firstHour)}`}</small>
      </button>;
    })}
  </div>;
}

export default { render: WarContributionView, countryChip: WarChip, subjectMenu: WarMenu, useOptionChoices: useWarChoices } satisfies FamilyViewRenderer;
