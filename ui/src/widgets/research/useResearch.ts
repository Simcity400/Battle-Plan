import { useCallback, useEffect, useMemo, useRef } from 'react';
import type { OptionChoices, WidgetRuntime } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { useHeadProduct } from '../../dashboard/headProduct.ts';
import { useProduct } from '../../dashboard/products.ts';
import { setToken } from '../../dashboard/options';
import { useChartTokens } from '../../charts/useChartTokens';
import type { ResearchData, ResearchHistory, ResearchSlotsAt } from './types';
import type { ResearchSlotsReply } from '../../generated/dashboard';
import { liveKey } from '../../charts/live.ts';
import { paintResearchSlots } from './researchSlots.ts';
import { researchFolderList, researchFolderLooks, validateResearchReply, validateResearchSlots } from './researchData';
import type { ResearchUse } from './researchData';

/** Assignment definitions, deduplicated by token and template index, and the tokens slot reads name. */
export function historyTokens(history: ResearchHistory): ResearchUse[] {
  const tokens = new Set<string>(), definitions = new Map<string, Set<number>>();
  for (const slot of history.slots) {
    for (const entry of slot.entries) definitions.set(entry.token, (definitions.get(entry.token) ?? new Set<number>()).add(entry.databaseIndex));
    for (const state of slot.states) if (state.token && !definitions.has(state.token)) tokens.add(state.token);
  }
  return [...tokens, ...[...definitions].flatMap(([token, indices]) => [...indices].map(index => [token, index] as const))];
}

/** The widget's track and country. */
export function researchTarget(widget: WidgetSpec, runtime: WidgetRuntime) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  return { track, tag };
}

/** The country's whole research history on the branch: one request for the recording, branch and country,
 * shared by every research widget showing it; never asked again as playback moves. */
export function useResearchReply(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const { track, tag } = researchTarget(widget, runtime);
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'research', params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<ResearchData>(runtime.client, request, active, validateResearchReply, track?.source) };
}

/** What each slot holds at the hour shown: pushed at the head while following, only when a read changes it (a render
 * only when `structure` of it changes), asked for the hour when pinned or hovered. Its numbers are painted under
 * `root` with no render: each frame while followed, progress from the slots' live cells and days left counted down
 * at the head; `shown` hears the hour they are of. `data.hour` is the hour the reply is shown for. */
export function useResearchSlots(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean, structure: (reply: ResearchSlotsReply) => string,
  root: { current: Element | null }, shown: (hour: number | null) => void, hoverHour: number | null = null) {
  const { tag, track } = researchTarget(widget, runtime);
  const params = useMemo(() => tag ? { tag } : null, [tag]);
  const latest = useRef<ResearchSlotsReply | undefined>(undefined);
  const painter = useRef<() => void>(() => {});
  const paint = useCallback((reply: ResearchSlotsReply) => { latest.current = reply; painter.current(); }, []);
  const now = useHeadProduct<ResearchSlotsReply>(runtime, widget, 'research-slots', params, { active, validate: validateResearchSlots, structure, paint, hoverHour });
  // The hour the numbers are counted at: the head while followed, else the hour asked.
  const headHour = useCallback(() => {
    if (!track) return null;
    if (now.read !== 'followed') return now.hour ?? now.at;
    return Math.max(track.branch.firstHour, Math.min(track.branch.lastHour, Math.floor(runtime.clock.getHour()) + track.axisOffset));
  }, [track, now.read, now.hour, now.at, runtime.clock]);
  painter.current = () => {
    const reply = now.read === 'followed' ? latest.current ?? now.data : now.data, hour = headHour();
    if (!reply || hour === null) return;
    const values = now.live.read();
    paintResearchSlots(root.current, reply, hour, slot => values.get(liveKey(String(slot), 'progress')));
    shown(now.shownAt() ?? hour);
  };
  useEffect(() => {
    if (!active) return;
    const repaint = () => painter.current();
    const stops = [runtime.clock.subscribeFrame(repaint), runtime.clock.subscribe(repaint), now.live.subscribe(repaint)];
    repaint();
    return () => { for (const stop of stops) stop(); };
  }, [active, runtime.clock, now.live, now.data]);
  const fallback = now.hour ?? now.at ?? track?.branch.lastHour ?? null;
  const data = useMemo<ResearchSlotsAt | undefined>(() => now.data && fallback !== null ? { ...now.data, hour: fallback } : undefined, [now.data, fallback]);
  return { ...now, data, latest, hourNow: () => headHour() ?? fallback ?? 0 };
}

/** The folder filter's choices: the folders this country's research shows, in
 * the content's order. */
export function useResearchChoices(widget: WidgetSpec, runtime: WidgetRuntime): OptionChoices {
  const { data } = useResearchReply(widget, runtime, true);
  const { categories } = useChartTokens();
  return useMemo(() => ({ folders: data ? researchFolderList(researchFolderLooks(data, historyTokens(data.history), categories))
    .map(look => ({ value: setToken(look.key), label: look.name })) : [] }), [data, categories]);
}
