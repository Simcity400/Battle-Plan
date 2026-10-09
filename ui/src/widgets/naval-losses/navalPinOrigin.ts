import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { setToken } from '../../dashboard/options';

/** A saved origin belongs only to the single country choice that created it. */
export function navalPinOrigin(widget: WidgetSpec, track?: Pick<BoundTrack, 'branch'>): { tag: string | null; hour: number | null } {
  const tag = widget.countries.kind === 'tags' && widget.countries.tags.length === 1 ? widget.countries.tags[0] : null;
  const offset = widget.options?.drillOriginOffset;
  if (widget.options?.show !== 'parts' || widget.options?.kind === 'ships' || widget.options?.kind === 'convoys'
    || offset == null || tag === null || widget.options?.drillOriginTag !== setToken(tag)) return { tag: null, hour: null };
  const hour = track && typeof offset === 'number' && Number.isInteger(offset) && offset >= 0 ? track.branch.firstHour + offset : null;
  return { tag, hour: hour !== null && track && hour >= track.branch.firstHour && hour <= track.branch.lastHour ? hour : null };
}
