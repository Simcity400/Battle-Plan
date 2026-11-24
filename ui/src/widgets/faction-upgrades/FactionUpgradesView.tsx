import { useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { countryFromKey } from '../../dashboard/countryKey';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDisplayNames, type NoFaction } from '../../dashboard/factionChoice';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { productRequest } from '../../dashboard/request';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import { useIdentitySnapshot } from '../../families/identity/react';
import FamilyTimelineHost from '../../families/timeline/FamilyTimelineHost';
import type { FactionRulesReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { factionUpgradesProjection, type FactionUpgradesProjectionOptions } from './factionUpgradesProjection';

const NO_SUBS = [] as const;

export default function FactionUpgradesView({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const { groups, pick, tag: viewpointTag, hour: chosenAt } = useChosenFaction(widget, runtime, track, active);
  const displayNames = useFactionDisplayNames(track?.source);
  const faction = pick?.faction ?? null;

  const req = useMemo(
    () => (track && faction
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'faction-state',
          params: {
            view: 'rules',
            branchId: track.branch.id,
            faction: faction.id,
            viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag),
          },
        })
      : null),
    [track, faction, viewpointTag],
  );
  const reply = useProduct<FactionRulesReply>(runtime.client, req, active, undefined, track?.source);
  const { snapshot: identitySnapshot, onDemands: onIdentityDemands } = useIdentitySnapshot(runtime.client, track, active);

  const factionName = faction && chosenAt !== null ? factionNameAt(faction, chosenAt, displayNames[faction.id]) : '';

  const projectionOptions = useMemo<FactionUpgradesProjectionOptions>(() => ({
    factionName,
    endedProgrammes: options.endedProgrammes !== 'hidden',
    branchFirst: track?.branch.firstHour ?? 0,
  }), [factionName, options.endedProgrammes, track?.branch.firstHour]);

  const staticSources = useMemo(() => (reply.data
    ? { panels: { main: reply.data }, hour: chosenAt ?? track?.branch.lastHour ?? 0 }
    : null), [reply.data, chosenAt, track?.branch.lastHour]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track],
  );

  const blocked = useMemo(() => {
    if (!track) return <WidgetState kind="empty" icon="history" message="Load a campaign to see faction rules." />;
    if (coverage.outside || chosenAt === null) return <WidgetState kind="outside" read={coverage} />;
    if (groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (!groups.data) return <WidgetState kind="loading" label="Loading faction rules…" />;
    if (!faction) return <WidgetState kind="empty" icon="history" message={noFactionMessage(pick!.reason as NoFaction, following)} />;
    if (reply.error) return <WidgetState kind="error" failure={reply.error} onRetry={reply.retry} />;
    if (!reply.data) return <WidgetState kind="loading" label="Loading faction rules…" />;
    return null;
  }, [track, coverage, chosenAt, groups.error, groups.data, groups.retry, faction, pick, following, reply.error, reply.retry, reply.data]);

  return (
    <FamilyTimelineHost
      declaration={factionUpgradesProjection}
      options={projectionOptions}
      label={factionName ? `${factionName} rules and upgrades` : 'Faction upgrades and rules'}
      subject="faction rules"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={NO_SUBS}
      allowEmptyFollow
      owner={track && faction ? `faction-upgrades:${track.source.id}:${track.branch.id}:${faction.id}` : 'faction-upgrades'}
      staticSources={staticSources}
      offset={track?.axisOffset ?? 0}
      pin={following ? null : (chosenAt ?? track?.branch.lastHour ?? null)}
      bounds={bounds}
      identitySnapshot={identitySnapshot}
      onIdentityDemands={onIdentityDemands}
      contextMenuItems={track ? (_key, _name, h) => [jumpItem(runtime, track.id, h)] : undefined}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
    />
  );
}
