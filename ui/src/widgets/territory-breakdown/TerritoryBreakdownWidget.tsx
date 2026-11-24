import { useCallback, useMemo } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { useFactionColours } from '../../dashboard/FactionColours';
import { resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import type { GatedHour } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { measureOf } from '../../dashboard/territoryMeasure.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import FamilyBreakdownHost from '../../families/breakdown/FamilyBreakdownHost.tsx';
import { breakdownViewOf, levelsShown } from '../../families/breakdown/options.ts';
import { useLaneIdentity } from '../../families/breakdown/useLaneIdentity.ts';
import type { FamilySourceSubscription } from '../../families/contract.ts';
import { validateBreakdownReply, type RegionalRule } from './breakdownData.ts';
import { LEVELS, OPTIONS } from './definition';
import { territoryBreakdownProjection, type TerritoryBreakdownProjectionOptions } from './territoryBreakdownProjection.ts';
import type { TerritoryBreakdownReply } from './types';

export default function TerritoryBreakdownWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const view = breakdownViewOf(options);
  const namesInRing = options.namesInRing !== false;
  const measure = measureOf(options.measure);
  const rule: RegionalRule = options.rule === 'owned' ? 'owned' : 'controlled';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';

  const resolvedWin = useMemo(
    () => !following && track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null,
    [following, track, widget, runtime.tracks, runtime.clock],
  );
  const pinHour = !following && resolvedWin ? Math.floor(resolvedWin.lastHour) : null;

  const staticReq = useMemo(
    () => !following && track && pinHour !== null
      ? productRequest({
          sourceId: track.source.id,
          revision: track.source.revision,
          product: 'territory-breakdown',
          params: { branchId: track.branch.id, hour: pinHour as GatedHour, measure, rule },
        })
      : null,
    [following, track?.source.id, track?.source.revision, track?.branch.id, pinHour, measure, rule],
  );
  const staticRead = useProduct<TerritoryBreakdownReply>(runtime.client, staticReq, active && !following, validateBreakdownReply, track?.source);

  const subscriptions = useMemo<readonly FamilySourceSubscription[]>(() => {
    if (!following || !track) return [];
    return [{
      name: 'breakdown',
      source: track.source,
      branch: track.branch.id,
      offset: track.axisOffset,
      channel: 'panel',
      product: 'territory-breakdown',
      query: { kind: 'territory-breakdown', measure, rule },
    }];
  }, [following, track, measure, rule]);

  const groups = useCountryGroups(runtime, track, active);
  const colours = useFactionColours(runtime.client, track?.source);
  const { identitySnapshot, onIdentityDemands } = useLaneIdentity(runtime.client, track, active, undefined, { needFlags: true, needGroups: true });

  const fallback = defaultCountryKey(track);
  const lastHour = track?.branch.lastHour ?? 0;
  const resolved = useMemo(
    () => widget.countries.kind === 'all'
      ? null
      : resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, Number.MAX_SAFE_INTEGER),
    [widget.countries, fallback, groups.data, lastHour],
  );

  const levelKey = LEVELS.map(level => options[level] !== false).join();
  const levels = useMemo(() => levelsShown(options, LEVELS), [levelKey]);

  const factionColours = useMemo(() => Object.fromEntries(colours.colours), [colours.colours]);
  const factionNames = useMemo(() => Object.fromEntries(colours.displayNames), [colours.displayNames]);

  const projectionOptions = useMemo<TerritoryBreakdownProjectionOptions>(() => ({
    measure,
    rule,
    levels,
    selectedTags: resolved ? resolved.tags : null,
    groups: groups.data ?? null,
    factionColours,
    factionNames,
    world: widget.countries.kind === 'all',
  }), [measure, rule, levels, resolved, groups.data, factionColours, factionNames, widget.countries.kind]);

  const staticSources = useMemo(() => {
    if (following || !staticRead.data || pinHour === null) return null;
    return {
      panels: { breakdown: staticRead.data },
      hour: pinHour,
    };
  }, [following, staticRead.data, pinHour]);

  const bounds = useMemo<readonly [number, number] | null>(
    () => track ? [track.branch.firstHour, track.branch.lastHour] : null,
    [track],
  );

  const contextMenuItems = useCallback(
    (_key: string | null, _name: string | null, atHour: number) => (track ? [jumpItem(runtime, track.id, atHour)] : []),
    [runtime, track],
  );

  const extraWarnings = useMemo(
    () => [...(groups.data?.warnings ?? []), ...colours.warnings],
    [groups.data?.warnings, colours.warnings],
  );

  const blocked = !track
    ? <WidgetState kind="empty" icon="sunburst" message="Load a campaign to see its territory breakdown." />
    : !following && staticRead.error
      ? <WidgetState kind="error" failure={staticRead.error} onRetry={staticRead.retry} />
      : groups.error && !groups.data
        ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
        : colours.error
          ? <WidgetState kind="error" what="faction colours" failure={colours.error} />
          : !groups.data || colours.loading || (!following && !staticRead.data)
            ? <WidgetState kind="loading" label="Loading territory breakdown…" />
            : resolved?.reason
              ? <WidgetState kind="unavailable" message={resolved.reason} />
              : null;

  return (
    <FamilyBreakdownHost
      declaration={territoryBreakdownProjection}
      options={projectionOptions}
      view={view}
      namesInRing={namesInRing}
      label="Territory breakdown"
      subject="territory breakdown"
      widget={widget}
      runtime={runtime}
      active={active}
      subscriptions={subscriptions}
      staticSources={staticSources}
      offset={track?.axisOffset ?? 0}
      pin={pinHour}
      bounds={bounds}
      identitySnapshot={identitySnapshot}
      onIdentityDemands={onIdentityDemands}
      contextMenuItems={contextMenuItems}
      onSetOption={setOption}
      onDiagnostics={onDiagnostics}
      extraWarnings={extraWarnings}
      blockedState={blocked}
      loadingLabel="Loading territory breakdown…"
    />
  );
}
