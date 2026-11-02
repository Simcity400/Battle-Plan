import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { useTableHour } from '../../charts/table/useTableHour';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { subjectHandle } from '../../dashboard/chosen';
import { pendingReplyAt } from '../../dashboard/preparedPrefix';
import { useFollowedProduct } from '../../dashboard/products';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { runAt } from '../../charts/series';
import { leadersAt, topLeader, validateUnitLeadersReply } from '../generals/generalsData';
import type { UnitLeadersData } from '../generals/generalsData';
import { useLeaderCountries } from '../generals/useLeaders';
import { OPTIONS } from './definition';
import CareerView from './CareerView';

export default function GeneralCareerWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
  if (!window) return <WidgetState kind="unavailable" message="The chosen moment is outside this campaign." />;
  if (!options.leader) return <TopGeneralCareer {...props} hour={Math.floor(window.lastHour)} />;
  const reference = subjectHandle(track.source.metadata.campaignKey, String(options.leader));
  if (reference === null) return <WidgetState kind="unavailable" message="The saved general is from another campaign. Choose one in widget settings." />;
  return <CareerView {...props} reference={reference} hour={Math.floor(window.lastHour)} />;
}

/** The first army leader of the widget's countries on the Generals leaderboard at its default sort. */
function TopGeneralCareer(props: WidgetProps & { hour: number }) {
  const { widget, runtime, active, hour } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const dashboard = widget.countries.kind === 'selected';
  const tag = dashboard ? defaultCountryTag(track) : null;
  const selection = useLeaderCountries(widget, runtime, hour, active);
  const request = track && selection.requestSelection ? { sourceId: track.source.id, revision: track.source.revision, product: 'unit-leaders',
    params: { branchId: track.branch.id, selection: selection.requestSelection, view: 'leaderboard', leaders: 'land', hour } } : null;
  // The last ranking stays shown while the next hour's is read.
  const { data, error, retry, moment } = useFollowedProduct<UnitLeadersData>(runtime.client, request, ['hour'], active, validateUnitLeadersReply, track?.source);
  const read = data && moment ? (JSON.parse(moment) as [number])[0] : hour;
  const rows = useMemo(() => data && selection.countries !== undefined ? leadersAt(data, read, selection.countries, 'land') : [],
    [data, read, selection.countries]);
  const top = useMemo(() => topLeader(rows), [rows]);
  const country = useCountryNameAt(runtime.client, track, tag, read, active);
  const nameDetail = dashboard && country.nameFailure ? <span data-detail={country.nameFailure.message}>{country.name}</span> : undefined;
  if (dashboard && !tag) return <WidgetState kind="empty" message="Choose a dashboard country or a leader in the widget settings." />;
  if (selection.group.error) return <WidgetState kind="error" failure={selection.group.error} onRetry={selection.group.retry} />;
  if (selection.selectionReason) return <WidgetState kind="unavailable" message={selection.selectionReason} />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data || selection.countries === undefined) return <WidgetState kind="loading" label="Loading leaders…" />;
  if (top) return <CareerView {...props} reference={top.key} caption={dashboard ? country.nameFailure ? <span data-detail={country.nameFailure.message}>{country.name}</span> : `Following ${country.name}’s top general.` : 'Following the top general of these countries.'} />;
  if (!selection.known || pendingReplyAt(data, read)) return <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  if (rows.length) return <WidgetState kind="unavailable" message="Generals’ skills weren’t recorded at this time." detail={nameDetail} />;
  if (!runAt(data.observed, read)?.leaders) return <WidgetState kind="unavailable" message="Leaders weren’t recorded at this time." detail={nameDetail} />;
  return <WidgetState kind="empty" message={nameDetail ? 'No generals yet.' : dashboard ? `${country.name} has no generals yet.` : 'These countries have no generals yet.'} detail={nameDetail} />;
}
