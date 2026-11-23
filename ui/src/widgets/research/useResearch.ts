import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useMemo } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct.ts';
import { useProduct } from '../../dashboard/products.ts';
import { setToken } from '../../dashboard/options';
import { useChartTokens } from '../../charts/useChartTokens';
import type { ResearchData, ResearchHistory, ResearchSlotsAt } from './types';
import type { ResearchSlotsReply } from '../../generated/dashboard';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import { panelFeed } from '../../dashboard/panelFeed';
import type { RefObject } from 'react';
import { researchFolderList, researchFolderLooks, validateResearchReply, validateResearchSlots } from './researchData';
import { inspectedSlot } from './researchSlots.ts';
import type { ResearchUse } from './researchData';

export function historyTokens(history: ResearchHistory): ResearchUse[] {
  const tokens = new Set<string>(), definitions = new Map<string, Set<number>>();
  for (const slot of history.slots) {
    for (const entry of slot.entries) definitions.set(entry.token, (definitions.get(entry.token) ?? new Set<number>()).add(entry.databaseIndex));
    for (const state of slot.states) if (state.token && !definitions.has(state.token)) tokens.add(state.token);
  }
  return [...tokens, ...[...definitions].flatMap(([token, indices]) => [...indices].map(index => [token, index] as const))];
}

export function researchTarget(widget: WidgetSpec, runtime: WidgetRuntime) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  return { track, tag };
}

export function useResearchReply(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const { track, tag } = researchTarget(widget, runtime);
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'research', params: { branchId: track.branch.id, country: countryFromKey(tag) } } : null;
  return { track, tag, ...useProduct<ResearchData>(runtime.client, request, active, validateResearchReply, track?.source) };
}

export function useResearchSlots(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, structure: (reply: ResearchSlotsReply) => string,
  root: RefObject<HTMLElement | null>, hoverHour: number | null = null) {
  const { tag, track } = researchTarget(widget, runtime);
  const params = useMemo(() => tag ? { country: countryFromKey(tag) } : null, [tag]);
  const now = useHeadProduct<ResearchSlotsReply>(runtime, widget, 'research-slots', params,
    { active, validate: validateResearchSlots, structure, hoverHour });
  const attach = useCallback((element: HTMLElement | null) => {
    root.current = element;
    now.root(element);
  }, [root, now.root]);
  const offset = track?.axisOffset ?? 0;
  const fixed = widget.time.mode !== 'follow' || hoverHour !== null;
  const shown = now;
  const fallback = shown.at;
  const headHour = () => fixed ? fallback : runtime.clock.getHour() + offset;
  const feed = panelFeed('research-slots', now, structure, widget, runtime);
  useDrawnPanel(root, runtime.clock, feed && hoverHour !== null ? { ...feed, fixed: hoverHour - offset } : feed, active);
  const latest = { get current() { const read = now.current(), hour = now.shownAt(); return read && hour !== null ? { ...read, hour } : undefined; },
    inspect: (slot: number) => inspectedSlot(now.inspected(), slot) };
  const data = useMemo<ResearchSlotsAt | undefined>(() => shown.data && fallback !== null ? { ...shown.data, hour: fallback } : undefined, [shown.data, fallback]);
  return { ...shown, root: attach, data, latest, feed, hourNow: () => headHour() ?? fallback ?? 0 };
}

export function useResearchChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResearchReply(widget, runtime, true);
  const { categories } = useChartTokens();
  return useMemo(() => ({ folders: data ? researchFolderList(researchFolderLooks(data, historyTokens(data.history), categories))
    .map(look => ({ value: setToken(look.key), label: look.name })) : [] }), [data, categories]);
}
