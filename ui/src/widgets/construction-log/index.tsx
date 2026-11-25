import { useCallback, useEffect, useMemo, useState } from 'react';
import { validateProductReply } from '../../dashboard/client.ts';
import type { FamilyViewProps, FamilyViewRenderer } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { resolveWindow } from '../../dashboard/model.ts';
import { contentKey, contentSet, setToken } from '../../dashboard/options.ts';
import { useGate } from '../../dashboard/regionGate.ts';
import { productRequest } from '../../dashboard/request.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import LogKindFilter from '../../families/table/LogKindFilter.tsx';
import type { ConstructionTypeChip } from '../../generated/dashboard';
import { constructionTypeFilterItems } from './constructionLogProjection.ts';

interface ChipsState {
  readonly scope: string;
  readonly hour: number;
  readonly nextChange: number | null;
  readonly chips: readonly ConstructionTypeChip[];
}

function validateBuildingTypeChips(data: unknown): { readonly chips: ConstructionTypeChip[]; readonly nextChange: number | null } {
  if (typeof data !== 'object' || data === null || Array.isArray(data)) {
    throw new Error('The construction log chips reply is malformed.');
  }
  const obj = data as Record<string, unknown>;
  if (!Array.isArray(obj.buildingTypes)) throw new Error('The construction log chips reply is malformed.');
  const chips: ConstructionTypeChip[] = [];
  for (const raw of obj.buildingTypes) {
    if (
      typeof raw !== 'object' || raw === null || Array.isArray(raw)
      || typeof (raw as Record<string, unknown>).token !== 'string'
      || !Number.isSafeInteger((raw as Record<string, unknown>).firstHour)
    ) {
      throw new Error('The construction log chips reply is malformed.');
    }
    const item = raw as Record<string, unknown>;
    chips.push({
      token: item.token as string,
      firstHour: item.firstHour as number,
      name: typeof item.name === 'string' ? item.name : null,
      icon: typeof item.icon === 'string' ? item.icon : null,
    });
  }
  const nextChange = Number.isSafeInteger(obj.nextChange) ? (obj.nextChange as number) : null;
  return { chips, nextChange };
}

function ConstructionLogView(props: FamilyViewProps) {
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const gate = useGate(props.runtime, props.widget);
  const shown = usePlaybackHour(props.runtime.clock);
  const scope = track ? `${track.source.id}:${track.source.revision}:${track.branch.id}` : '';
  const [chipsState, setChipsState] = useState<ChipsState | null>(null);

  const displayedHour = useMemo(() => {
    if (!track) return null;
    const following = props.widget.time.mode === 'follow';
    const win = !following ? resolveWindow(props.widget, track, [track], track.branch.lastHour) : null;
    const raw = following ? Math.floor(shown - track.axisOffset) : (win ? Math.floor(win.lastHour) : null);
    if (raw === null || !Number.isFinite(raw) || raw < track.branch.firstHour) return null;
    const clamped = Math.min(track.branch.lastHour, raw);
    return gate.recorded(clamped);
  }, [track, props.widget, shown, gate]);

  const needsFetch = useMemo(() => {
    if (!track || !props.active || displayedHour === null) return false;
    if (!chipsState || chipsState.scope !== scope) return true;
    if (displayedHour < chipsState.hour) return true;
    return chipsState.nextChange !== null && displayedHour >= chipsState.nextChange;
  }, [track, props.active, displayedHour, chipsState, scope]);

  useEffect(() => {
    if (!track || !needsFetch || displayedHour === null) return;
    const controller = new AbortController();
    const request = productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'construction-log-tail',
      params: { branchId: track.branch.id, hour: displayedHour, limit: 1 },
    });
    props.runtime.client.product(request, controller.signal).then(reply => {
      if (controller.signal.aborted) return;
      const parsed = validateBuildingTypeChips(validateProductReply(reply, request).data);
      setChipsState({ scope, hour: displayedHour, nextChange: parsed.nextChange, chips: parsed.chips });
    }).catch(() => {});
    return () => controller.abort();
  }, [track, needsFetch, displayedHour, props.runtime.client, scope]);

  const selectedTypes = useMemo(
    () => contentSet(props.options.types ?? '').map(contentKey),
    [props.options.types],
  );

  const visibleChips = chipsState && chipsState.scope === scope ? chipsState.chips : [];
  const filterItems = useMemo(
    () => constructionTypeFilterItems(visibleChips, selectedTypes, displayedHour ?? undefined),
    [visibleChips, selectedTypes, displayedHour],
  );

  const onToggle = useCallback((token: string, nextChecked: boolean) => {
    const visibleValues = filterItems.map(item => item.value);
    const current = selectedTypes.length === 0 ? new Set(visibleValues) : new Set(selectedTypes);
    if (nextChecked) current.add(token);
    else current.delete(token);
    const allVisibleChecked = visibleValues.length > 0 && visibleValues.every(value => current.has(value));
    if (current.size === 0 || allVisibleChecked) {
      props.setOption?.('types', '');
      return;
    }
    props.setOption?.('types', [...current].map(setToken).join(' '));
  }, [filterItems, selectedTypes, props.setOption]);

  const onSelectAll = useCallback(() => {
    props.setOption?.('types', '');
  }, [props.setOption]);

  return (
    <>
      <LogKindFilter
        slot={props.filterSlot}
        allLabel="All buildings"
        noun="buildings"
        items={filterItems}
        onToggle={onToggle}
        onSelectAll={onSelectAll}
      />
      {props.manifestDeclaration ? (
        <FamilyWidget
          {...props}
          family={props.manifestDeclaration.family}
          declaration={props.manifestDeclaration}
        />
      ) : null}
    </>
  );
}

export default { render: ConstructionLogView } satisfies FamilyViewRenderer;
