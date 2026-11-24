import { useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { countryFromKey } from '../../dashboard/countryKey';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { useTimelineIdentity } from '../../families/timeline/useTimelineIdentity';
import FamilyTimelineHost from '../../families/timeline/FamilyTimelineHost';
import type { WarContributionsReply, WarLossesReply, WarsReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import { validateWarContributions, validateWarLosses, validateWarsReply, type WarsColour, type WarsView as WarsViewMode } from './warsData';
import { warsProjection, type WarsProjectionOptions } from './warsProjection';

const NO_SUBS = [] as const;
const SURRENDER_SPRITE = [{ key: 'GFX_surrender_tiny_flag', frame: 0 }] as const;

export default function WarsView({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const gate = useGate(runtime, widget, SPEC, active);
  const viewpointTag = selectedCountryKeys(widget.countries, track)[0] ?? null;

  const req = useMemo(
    () => (track
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'wars',
          params: { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag) },
        })
      : null),
    [track, viewpointTag],
  );
  const product = useProduct<WarsReply>(runtime.client, req, active, validateWarsReply, track?.source);
  const data = product.data;

  const spriteIcon = useSpriteArtwork(runtime, track, SURRENDER_SPRITE, active);
  const surrenderIconUrl = spriteIcon(SURRENDER_SPRITE[0]);

  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const hoverAt = gate.recorded(hoverHour);
  const hoverParams = useMemo(
    () => (track && hoverAt !== null
      ? { branchId: track.branch.id, viewpoint: viewpointTag === null ? null : countryFromKey(viewpointTag), hour: hoverAt }
      : null),
    [track, hoverAt, viewpointTag],
  );
  const losses = useProduct<WarLossesReply>(
    runtime.client,
    track && hoverParams ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'war-losses', params: hoverParams }) : null,
    active && hoverParams !== null,
    validateWarLosses,
    track?.source,
  );
  const contributions = useProduct<WarContributionsReply>(
    runtime.client,
    track && hoverParams ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'war-contributions', params: hoverParams }) : null,
    active && hoverParams !== null,
    validateWarContributions,
    track?.source,
  );

  const hoverPanels = useMemo(() => {
    if (hoverAt === null) return undefined;
    const out: Record<string, { readonly hour: number; readonly data: unknown }> = {};
    if (losses.data) out['war-losses'] = { hour: hoverAt, data: losses.data };
    if (contributions.data) out['war-contributions'] = { hour: hoverAt, data: contributions.data };
    return Object.keys(out).length ? out : undefined;
  }, [hoverAt, losses.data, contributions.data]);

  const { snapshot: identitySnapshot, onDemands: onIdentityDemands } = useTimelineIdentity(runtime.client, track, active);

  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const head = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);

  const projectionOptions = useMemo<WarsProjectionOptions>(() => ({
    view: (options.view as WarsViewMode) === 'countries' ? 'countries' : 'gantt',
    colour: (options.colour as WarsColour) === 'side' ? 'side' : 'war',
    branchFirst: track?.branch.firstHour ?? 0,
    surface: '#16181d',
    surrenderIconUrl,
  }), [options.view, options.colour, track?.branch.firstHour, surrenderIconUrl]);

  const staticSources = useMemo(() => (data
    ? { panels: { main: data }, hour: head, provenInterval: following && track ? [track.branch.firstHour, track.branch.lastHour] as const : null }
    : null), [data, head, following, track]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track],
  );

  const blocked = useMemo(() => {
    if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its wars." />;
    if (widget.time.mode !== 'follow' && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
    if (product.error) return <WidgetState kind="error" failure={product.error} onRetry={product.retry} />;
    if (!data) return <WidgetState kind="loading" label="Loading wars…" />;
    return null;
  }, [track, widget.time.mode, window, product.error, product.retry, data]);

  return (
    <FamilyTimelineHost
      declaration={warsProjection}
      options={projectionOptions}
      label="Wars timeline"
      subject="wars"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={NO_SUBS}
      allowEmptyFollow
      owner={track ? `wars:${track.source.id}:${track.branch.id}:${viewpointTag ?? ''}` : 'wars'}
      staticSources={staticSources}
      hoverPanels={hoverPanels}
      offset={track?.axisOffset ?? 0}
      pin={following ? null : head}
      bounds={bounds}
      identitySnapshot={identitySnapshot}
      onIdentityDemands={onIdentityDemands}
      onHoverHour={setHoverHour}
      contextMenuItems={track ? (_key, _name, h) => [jumpItem(runtime, track.id, h)] : undefined}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      blockedState={blocked}
    />
  );
}
