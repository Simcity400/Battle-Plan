import { useCallback, useMemo } from 'react';
import { dayLabel } from '../../charts/draw';
import { CountryFlag } from '../../components/CountryFlag';
import type { WidgetCountryChipProps, WidgetProps, WidgetRenderer, WidgetSubjectMenuProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry';
import { useIdentity, useIdentityScope } from '../../families/identity/react.ts';
import { countryTagKey } from '../../families/identity/types.ts';
import { useHeadHour } from '../army-size/headHour';
import { labelObservationAt } from '../wars/warsData';
import { MEASURE_VIEW, warContributionMeasureDeclaration } from './measureProjection.ts';
import { warContributionSidesDeclaration } from './sidesProjection.ts';
import { chosenWar } from './warContributionData.ts';
import { savedWar } from './warContributionPlan.ts';
import { useWarChoices, useWars } from './useWarChoices';
import './warContribution.css';

function WarContributionView(props: WidgetProps) {
  const { setOption } = props;
  const onActivate = useCallback((action: InstalledAction) => {
    const pick = (action.meta as { readonly pick?: string } | undefined)?.pick;
    if (pick) setOption?.('measure', pick);
  }, [setOption]);
  return props.view === MEASURE_VIEW
    ? <FamilyWidget {...props} family="bars" view={MEASURE_VIEW} declaration={warContributionMeasureDeclaration} onActivate={onActivate} />
    : <FamilyWidget {...props} family="table" view="sides" declaration={warContributionSidesDeclaration} />;
}

function useChosenWar(props: Pick<WidgetProps, 'widget' | 'runtime' | 'active'>) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const wars = useWars(widget, runtime, active);
  const saved = savedWar(widget.options ?? {});
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

function useLeader(props: Pick<WidgetProps, 'runtime' | 'active'>, track: ReturnType<typeof useChosenWar>['track'], tag: string | undefined, hour: number | null) {
  const scope = useIdentityScope(props.runtime.client, track?.source, track?.branch.id ?? null, props.active && !!tag);
  const key = useMemo(() => (tag ? countryTagKey(tag) : null), [tag]);
  const { result, flag, flagArt } = useIdentity(props.runtime.client, track?.source, scope, key, hour, { active: props.active && !!tag, needFlag: true, flagSize: 'small' });
  const name = result?.kind === 'known' && result.name.kind === 'known' ? result.name.value : tag ?? '';
  return { name, flag, pending: flagArt.status === 'pending' };
}

function WarChip(props: WidgetCountryChipProps) {
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

function WarMenu({ widget, runtime, onApply }: WidgetSubjectMenuProps) {
  const wars = useWars(widget, runtime, true);
  const saved = savedWar(widget.options ?? {});
  const choose = (id: string | null) => onApply({ ...widget, options: { ...(widget.options ?? {}), war: id === null ? '' : setToken(id) } });
  return <div className="war-menu" role="group" aria-label="War">
    <div className="war-menu-heading">War</div>
    <button type="button" className={`war-menu-row${saved === null ? ' is-chosen' : ''}`} onClick={() => choose(null)}>The country’s main war at the date</button>
    {(wars?.wars ?? []).map(war => {
      const observation = war.observations.at(-1) ?? null;
      const name = observation?.name.known ? observation.name.text : 'War name not recorded';
      return <button key={war.id} type="button" className={`war-menu-row${saved === war.id ? ' is-chosen' : ''}`} onClick={() => choose(war.id)}>
        <span>{name}</span><small>{`${war.startedBy ? 'By ' : 'From '}${dayLabel(war.firstHour)}`}</small>
      </button>;
    })}
  </div>;
}

export default { render: WarContributionView, countryChip: WarChip, subjectMenu: WarMenu, useOptionChoices: useWarChoices } satisfies WidgetRenderer;
