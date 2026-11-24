import { useCallback, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { CountryFlag } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { contentKey, resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { productRequest } from '../../dashboard/request.ts';
import { useHeadHour } from '../army-size/headHour';
import type { WarContributionsReply, WarsReply } from '../../generated/dashboard';
import { validateWarContributions, validateWarsReply } from '../wars/warsData';
import { useLaneIdentity } from '../../families/breakdown/useLaneIdentity.ts';
import type { FamilySourceSubscription } from '../../families/contract.ts';
import { useIdentity, useIdentityScope, countryTagKey } from '../../families/identity/react.ts';
import FamilyWarContributionHost from '../../families/war-contribution/FamilyWarContributionHost.tsx';
import type { WCShowMode, WCViewMode } from '../../families/war-contribution/model.ts';
import { OPTIONS } from './definition';
import { chosenWar, readHourFor } from './warContributionData';

export default function WarContributionWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view: WCViewMode = options.view === 'measure' ? 'measure' : 'sides';
  const show: WCShowMode = options.show === 'totals' ? 'totals' : 'share';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;

  const structuralReq = useMemo(
    () => track
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'wars',
          params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) },
        })
      : null,
    [track?.source.id, track?.source.revision, track?.branch.id, viewpointTag],
  );
  const structural = useProduct<WarsReply>(runtime.client, structuralReq, active, validateWarsReply, track?.source);
  const saved = typeof options.war === 'string' && options.war ? contentKey(options.war) : null;

  const trackHour = useCallback((clockHour: number) => {
    const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
    return window ? Math.floor(window.lastHour) : null;
  }, [track, widget, runtime.tracks]);

  const clockHour = useHeadHour(runtime.clock, active && following, hour => {
    const at = trackHour(hour);
    const war = at === null || !structural.data ? null : chosenWar(structural.data, saved, at);
    if (at === null || !war) return String(at === null ? '' : 'none');
    return `${war.id}:${war.observations.findIndex(item => item.firstHour <= at && at <= item.lastHour)}:${at > war.lastHour}`;
  });

  const window = track && resolveWindow(widget, track, runtime.tracks, clockHour);
  const at = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const war = structural.data ? chosenWar(structural.data, saved, at) : null;
  const readHour = war ? readHourFor(war, at) : null;
  const going = following && war !== null && readHour === at;

  const gate = useGate(runtime, widget);
  const endedAt = gate.recorded(readHour);
  const endedReq = useMemo(
    () => track && war && !going && endedAt !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'war-contributions',
          params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag), hour: endedAt },
        })
      : null,
    [track?.source.id, track?.source.revision, track?.branch.id, war, going, endedAt, viewpointTag],
  );
  const ended = useProduct<WarContributionsReply>(runtime.client, endedReq, active && !going, validateWarContributions, track?.source);

  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(() => {
    if (!going || !track) return [];
    return [{
      name: 'contributions',
      source: track.source,
      branch: track.branch.id,
      offset: track.axisOffset,
      channel: 'panel',
      product: 'war-contributions',
      query: { kind: 'war-contributions', viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) },
    }];
  }, [going, track, viewpointTag]);

  const { identitySnapshot, onIdentityDemands } = useLaneIdentity(runtime.client, track, active);

  const [headerMeta, setHeaderMeta] = useState<{
    warTitle: string;
    leftLeadTag: string | null;
    rightLeadTag: string | null;
    leftLeadName: string;
    rightLeadName: string;
  } | null>(null);

  const scope = useIdentityScope(runtime.client, track?.source, track?.branch.id, active && Boolean(headerSlot && headerMeta));
  const leftKey = useMemo(() => headerMeta?.leftLeadTag ? countryTagKey(headerMeta.leftLeadTag) : null, [headerMeta?.leftLeadTag]);
  const rightKey = useMemo(() => headerMeta?.rightLeadTag ? countryTagKey(headerMeta.rightLeadTag) : null, [headerMeta?.rightLeadTag]);
  const leftLead = useIdentity(runtime.client, track?.source, scope, leftKey, at, { active: active && Boolean(headerSlot && leftKey), needFlag: true });
  const rightLead = useIdentity(runtime.client, track?.source, scope, rightKey, at, { active: active && Boolean(headerSlot && rightKey), needFlag: true });

  const header = headerSlot && headerMeta && war && createPortal(
    <span className="wc-war-subject" data-tooltip={headerMeta.warTitle}>
      {headerMeta.leftLeadTag && (
        <CountryFlag flag={leftLead.flag?.image} pending={leftLead.flagArt.status === 'pending'} tag={headerMeta.leftLeadTag} />
      )}
      <span className="wc-war-subject-sep">–</span>
      {headerMeta.rightLeadTag && (
        <CountryFlag flag={rightLead.flag?.image} pending={rightLead.flagArt.status === 'pending'} tag={headerMeta.rightLeadTag} />
      )}
      <span>{headerMeta.warTitle}</span>
    </span>,
    headerSlot,
  );

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const contextMenuItems = useCallback(() => {
    if (!war || !track) return [];
    return [{ ...jumpItem(runtime, track.id, war.firstHour), label: `Jump to war start ${dayLabel(war.firstHour)}` }];
  }, [war, track, runtime]);

  const blocked = !track
    ? <WidgetState kind="empty" icon="swords" message="Load a campaign to see war contributions." />
    : !window
      ? <WidgetState kind="outside" />
      : structural.error
        ? <WidgetState kind="error" failure={structural.error} onRetry={structural.retry} />
        : !structural.data
          ? <WidgetState kind="loading" label="Loading war contributions…" />
          : !structural.data.available
            ? <WidgetState kind="unavailable" message="Wars are not in this recording." />
            : !war
              ? <WidgetState kind="empty" icon="swords" message={following ? 'This country has fought no war yet.' : 'This country has fought no war in this campaign.'} />
              : !going && ended.error
                ? <WidgetState kind="error" failure={ended.error} onRetry={ended.retry} />
                : !going && !ended.data
                  ? <WidgetState kind="loading" label="Loading war contributions…" />
                  : null;

  return (
    <>
      {header}
      <FamilyWarContributionHost
        view={view}
        show={show}
        war={war}
        widget={widget}
        runtime={runtime}
        active={active}
        subscriptions={subscriptions}
        staticReply={!going ? (ended.data ?? null) : null}
        offset={track?.axisOffset ?? 0}
        pin={going ? null : (readHour ?? at)}
        bounds={bounds}
        identitySnapshot={identitySnapshot}
        onIdentityDemands={onIdentityDemands}
        contextMenuItems={contextMenuItems}
        onDiagnostics={onDiagnostics}
        extraWarnings={structural.data?.warnings}
        blockedState={blocked}
        loadingLabel="Loading war contributions…"
        onHeaderMeta={setHeaderMeta}
      />
    </>
  );
}
