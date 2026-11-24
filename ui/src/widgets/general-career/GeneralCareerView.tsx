import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { subjectHandle } from '../../dashboard/chosen';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { SPEC, type RegionAction } from '../../dashboard/regionTime';
import { useGate } from '../../dashboard/regionGate';
import { productRequest } from '../../dashboard/request';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useTimelineIdentity } from '../../families/timeline/useTimelineIdentity';
import FamilyTimelineHost from '../../families/timeline/FamilyTimelineHost';
import type { LeaderCareerReply } from '../../generated/dashboard';
import { leaderRequestSelection, leaderRows, leadersStructure, topLeader, validateCareer, validateLeaderHour, validateLeaders, withheldLeaderState, withheldLeaders } from '../generals/generalsData';
import { useLeaderPortraits, type PortraitAt } from '../generals/portraits';
import { OPTIONS } from './definition';
import { generalCareerProjection, type GeneralCareerProjectionOptions, type GeneralCareerViewMode } from './generalCareerProjection';

const NO_SUBS = [] as const;
const NO_PORTRAIT: readonly PortraitAt[] = [];

export function GeneralCareerForReference({
  reference,
  time = SPEC,
  onPick,
  widget,
  runtime,
  active,
  headerSlot,
  onDiagnostics,
}: WidgetProps & { readonly reference: string; readonly time?: RegionAction; readonly onPick?: (hour: number) => void }) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view: GeneralCareerViewMode = options.view === 'dossier' ? 'dossier' : options.view === 'record' ? 'record' : 'timeline';
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';

  const careerReq = useMemo(
    () => (track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'unit-leader-career', params: { branchId: track.branch.id, reference } }) : null),
    [track, reference],
  );
  const career = useProduct<LeaderCareerReply>(runtime.client, careerReq, active, validateCareer, track?.source);
  const hourParams = useMemo(() => ({ reference }), [reference]);
  const head = useHeadProduct(runtime, useGate(runtime, widget, time, active), 'unit-leader-hour', hourParams, { active, validate: validateLeaderHour });

  const portraitAt = useMemo<readonly PortraitAt[]>(
    () => (career.data?.portraitHour != null ? [{ reference, hour: career.data.portraitHour }] : NO_PORTRAIT),
    [career.data?.portraitHour, reference],
  );
  const portraits = useLeaderPortraits(runtime, track, portraitAt, 118, active);
  const portrait = portraitAt[0] ? portraits.at(portraitAt[0]) : undefined;

  const { snapshot: identitySnapshot, onDemands: onIdentityDemands } = useTimelineIdentity(runtime.client, track, active);

  const portraitImage = portrait && !(portrait instanceof Error) ? portrait.image : null;
  const projectionOptions = useMemo<GeneralCareerProjectionOptions>(() => ({
    view,
    reference,
    branchFirst: track?.branch.firstHour ?? 0,
    surface: '#16181d',
    portraitUrl: portraitImage,
    portraitMissing: portrait !== undefined && !portraitImage,
  }), [view, reference, track?.branch.firstHour, portrait, portraitImage]);

  const shownHour = head.at ?? head.hour ?? track?.branch.lastHour ?? 0;
  const staticSources = useMemo(() => {
    if (!career.data) return null;
    const panels: Record<string, unknown> = { career: career.data };
    if (head.data) panels.hour = head.data;
    return {
      panels,
      hour: shownHour,
      provenInterval: following && track ? [track.branch.firstHour, track.branch.lastHour] as const : null,
    };
  }, [career.data, head.data, shownHour, following, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track],
  );

  const blocked = useMemo(() => {
    if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
    if (head.outside) return <WidgetState kind="outside" read={head} />;
    if (career.error) return <WidgetState kind="error" failure={career.error} onRetry={career.retry} />;
    if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
    if (!career.data) return <WidgetState kind="loading" label="Loading leader career…" />;
    return null;
  }, [track, head, career.error, career.retry, career.data]);

  return (
    <div ref={head.root} style={{ width: '100%', height: '100%', display: 'contents' }}>
      <FamilyTimelineHost
        declaration={generalCareerProjection}
        options={projectionOptions}
        label="General career"
        subject="leader career"
        widget={widget}
        runtime={runtime}
        active={active}
        subscriptions={NO_SUBS}
        allowEmptyFollow
        owner={track ? `general-career:${track.source.id}:${track.branch.id}:${reference}:${view}` : 'general-career'}
        staticSources={staticSources}
        offset={track?.axisOffset ?? 0}
        pin={following ? null : shownHour}
        bounds={bounds}
        identitySnapshot={identitySnapshot}
        onIdentityDemands={onIdentityDemands}
        onActivate={onPick ? (_key, _meta, hour) => { if (typeof hour === 'number') onPick(hour); } : undefined}
        contextMenuItems={track ? (_key, _name, h) => [jumpItem(runtime, track.id, h)] : undefined}
        headerSlot={headerSlot}
        onDiagnostics={onDiagnostics}
        extraWarnings={portraits.warnings}
        blockedState={blocked}
      />
    </div>
  );
}

export default function GeneralCareerView(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  if (!track) return <WidgetState kind="empty" message="Load a campaign to see a leader career." />;
  if (options.leader) {
    const reference = subjectHandle(track.source.metadata.campaignKey, String(options.leader));
    if (reference === null) {
      return <WidgetState kind="unavailable" message="The saved general is from another campaign. Choose one in widget settings." />;
    }
    return <GeneralCareerForReference {...props} reference={reference} />;
  }
  return <TopGeneralCareerView {...props} active={active} />;
}

function TopGeneralCareerView(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const dashboard = widget.countries.kind === 'selected';
  const tag = dashboard ? defaultCountryKey(track) : null;
  const atRead = useCountriesSelectionAt(widget, runtime, active);
  const params = useMemo(
    () => (atRead.requestTags === undefined ? null : { selection: leaderRequestSelection(atRead.requestTags), leaders: 'land' }),
    [atRead.requestTags],
  );
  const head = useHeadProduct(runtime, atRead.gate, 'unit-leaders', params, { active, validate: validateLeaders, structure: leadersStructure });
  const read = atRead.outside ? null : head.at ?? atRead.hour;
  const landed = read === null ? { countries: atRead.countries, reason: null } : atRead.countriesAt(read);
  const readCountries = landed.countries;
  const rows = useMemo(
    () => (head.data && read !== null && readCountries !== undefined ? leaderRows(head.data, read, readCountries, 'land') : []),
    [head.data, read, readCountries],
  );
  const top = useMemo(() => topLeader(rows), [rows]);

  if (dashboard && !tag) return <WidgetState kind="empty" message="Choose a dashboard country or a leader in the widget settings." />;
  if (head.outside || atRead.outside) return <WidgetState kind="outside" read={head} />;
  if (atRead.group.error) return <WidgetState kind="error" failure={atRead.group.error} onRetry={atRead.group.retry} />;
  if (atRead.selectionReason) return <WidgetState kind="unavailable" message={atRead.selectionReason} />;
  if (landed.reason) return <WidgetState kind="unavailable" message={landed.reason} />;
  if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  if (!head.data || readCountries === undefined) return <div ref={head.root}><WidgetState kind="loading" label="Loading leaders…" /></div>;
  if (top) return <GeneralCareerForReference {...props} reference={top.key} />;
  if (head.data.missing) return <WidgetState kind="unavailable" message="Leaders weren’t recorded at this time." detail={head.data.missing} />;
  if (rows.length) return <WidgetState kind="unavailable" message="Generals’ skills weren’t recorded at this time." />;
  const withheld = withheldLeaders(head.data, readCountries, 'land');
  if (withheld.length) {
    const state = withheldLeaderState(withheld);
    return <WidgetState kind="unavailable" message={state.message} detail={state.detail} />;
  }
  return <WidgetState kind="empty" message="No generals yet." />;
}
