import { defaultCountryTag } from './countrySelection';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { useProduct } from './products';
import type { IdeaHistoryReply } from '../generated/dashboard';
import { validateIdeaHistoryReply } from './ideaHistoryReply';

/** Which ideas a widget's country held, from the `idea-history` product: shared by
 * the idea timelines and the national spirits widgets and their settings. */
export function useIdeaHistory(widget: WidgetSpec, runtime: WidgetRuntime, active: boolean) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'idea-history',
    params: { branchId: track.branch.id, tag } } : null;
  return { track, tag, ...useProduct<IdeaHistoryReply>(runtime.client, request, active, validateIdeaHistoryReply) };
}
