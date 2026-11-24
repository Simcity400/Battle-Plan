import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { resolveOptions } from '../../dashboard/options';
import { subjectHandle } from '../../dashboard/chosen';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { leaderRequestSelection, leaderRows, leadersStructure, topLeader, validateLeaders, withheldLeaderState, withheldLeaders } from '../generals/generalsData';
import { OPTIONS } from './definition';
import CareerView from './CareerView';

export default function GeneralCareerWidget(props: WidgetProps) {
  const { widget, runtime } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
  if (!options.leader) return <TopGeneralCareer {...props} />;
  const reference = subjectHandle(track.source.metadata.campaignKey, String(options.leader));
  if (reference === null) return <WidgetState kind="unavailable" message="The saved general is from another campaign. Choose one in widget settings." />;
  return <CareerView {...props} grow={options.growWithPlayback === true} reference={reference} />;
}

function TopGeneralCareer(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const dashboard = widget.countries.kind === 'selected';
  const tag = dashboard ? defaultCountryKey(track) : null;
  const atRead = useCountriesSelectionAt(widget, runtime, active);
  const params = useMemo(() => atRead.requestTags === undefined ? null : { selection: leaderRequestSelection(atRead.requestTags), leaders: 'land' },
    [atRead.requestTags]);
  const head = useHeadProduct(runtime, atRead.gate, 'unit-leaders', params, { active, validate: validateLeaders, structure: leadersStructure });
  const read = atRead.outside ? null : head.at ?? atRead.hour;
  const landed = read === null ? { countries: atRead.countries, reason: null } : atRead.countriesAt(read);
  const readCountries = landed.countries;
  const rows = useMemo(() => head.data && read !== null && readCountries !== undefined ? leaderRows(head.data, read, readCountries, 'land') : [], [head.data, read, readCountries]);
  const top = useMemo(() => topLeader(rows), [rows]);
  const country = useCountryNameAt(runtime.client, track, tag, read, active);
  const nameDetail = dashboard && country.nameFailure ? <span data-detail={country.nameFailure.message}>{country.name}</span> : undefined;
  const body = () => {
    if (dashboard && !tag) return <WidgetState kind="empty" message="Choose a dashboard country or a leader in the widget settings." />;
    if (head.outside || atRead.outside) return <WidgetState kind="outside" read={head} />;
    if (atRead.group.error) return <WidgetState kind="error" failure={atRead.group.error} onRetry={atRead.group.retry} />;
    if (atRead.selectionReason) return <WidgetState kind="unavailable" message={atRead.selectionReason} />;
    if (landed.reason) return <WidgetState kind="unavailable" message={landed.reason} />;
    if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
    if (!head.data || readCountries === undefined) return <WidgetState kind="loading" label="Loading leaders…" />;
    if (top) return <CareerView {...props} grow={resolveOptions(OPTIONS, widget.options).growWithPlayback === true} reference={top.key} caption={dashboard ? nameDetail ?? `Following ${country.name}’s top general.` : 'Following the top general of these countries.'} />;
    if (head.data.missing) return <WidgetState kind="unavailable" message="Leaders weren’t recorded at this time." detail={nameDetail ?? head.data.missing} />;
    if (rows.length) return <WidgetState kind="unavailable" message="Generals’ skills weren’t recorded at this time." detail={nameDetail} />;
    const withheld = withheldLeaders(head.data, readCountries, 'land');
    if (withheld.length) {
      const state = withheldLeaderState(withheld, dashboard && !nameDetail ? country.name : undefined);
      return <WidgetState kind="unavailable" message={state.message} detail={nameDetail ? <>{nameDetail}{'\n'}{state.detail}</> : state.detail} />;
    }
    return <WidgetState kind="empty" message={nameDetail ? 'No generals yet.' : dashboard ? `${country.name} has no generals yet.` : 'These countries have no generals yet.'} detail={nameDetail} />;
  };
  return <div ref={head.root} className="general-career-top">{body()}</div>;
}
