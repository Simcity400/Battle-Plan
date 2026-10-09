import type { StackRun } from '../../charts/stack';
import type { DrillAction } from '../../charts/drill/drillState';
import type { FlowHit } from '../../charts/flow/FlowDiagram';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { resolveWindow } from '../../dashboard/model.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { TransferData, TransferLink, TransferMeasure, TransferOccurrence } from './territoryData';

export function transferTime(widget: WidgetSpec, track: BoundTrack, tracks: BoundTrack[], position: number, fromOffset: number | null = null) {
  const window = resolveWindow(widget, track, tracks, position);
  const from = Math.max(widget.time.mode === 'range' ? Math.ceil(window?.firstHour ?? Infinity) : -Infinity,
    fromOffset == null ? -Infinity : track.branch.firstHour + fromOffset);
  return { moment: Math.floor(window?.lastHour ?? -Infinity), from };
}

/** A held main view opens at its drawn moment; only Follow inherits the clock. */
export function transferDrillOpen(hour: number, following: boolean, playhead: number | null, playing: boolean): DrillAction {
  return { type: 'open', breakdown: 'parts', hour, playhead: following ? playhead : null, playing: following && playing };
}

export interface TransferDrawnSelection { transferIds: readonly string[]; transferConference: string | null }
export const transferDrawnSelection = (ids: readonly string[], conference: string | null): TransferDrawnSelection => ({ transferIds: ids, transferConference: conference });
export function transferHitSelection(hit: FlowHit): TransferDrawnSelection | null {
  const value = hit.kind === 'node' ? hit.node : hit.link;
  const drawn = value as typeof value & Partial<TransferDrawnSelection>;
  return drawn.transferIds && drawn.transferConference !== undefined ? drawn as TransferDrawnSelection : null;
}
const NO_LINKS: readonly TransferLink[] = [];
export const transferScopeLinks = (data: Pick<TransferData, 'campaign' | 'conferences'>, conference: string | null): readonly TransferLink[] => conference == null ? data.campaign : data.conferences.get(conference) ?? NO_LINKS;
/** Playback and content changes keep local row selection; a new raw cohort resets it. */
export const transferDrillKey = (source: string, branch: string, conference: string | null, ids: readonly string[]) => JSON.stringify([source, branch, conference, [...new Set(ids)].sort()]);
export function transferRestoreFocus(focused: Node | null, body: Node, anchor: Pick<Node, 'contains'> | null, panel: Pick<Node, 'contains'> | null): boolean {
  return focused == null || focused === body || !!anchor?.contains(focused) || !!panel?.contains(focused);
}

export const transferSelectionToken = (campaign: string, branch: string, ids: readonly string[], fromOffset: number | null = null) => setToken(campaignToken(campaign, JSON.stringify([branch, ids, fromOffset])));
export interface TransferSelection { ids: string[]; fromOffset: number | null }
export function transferSelection(token: string, campaign: string, branch: string): TransferSelection | null {
  if (!token) return { ids: [], fromOffset: null };
  const decoded = contentKey(token), prefix = `${campaign}:`;
  if (!decoded.startsWith(prefix)) return null;
  try {
    const value = JSON.parse(decoded.slice(prefix.length));
    return Array.isArray(value) && value.length === 3 && value[0] === branch && Array.isArray(value[1]) && value[1].every(id => typeof id === 'string') && (value[2] === null || Number.isSafeInteger(value[2]))
      ? { ids: [...new Set<string>(value[1])], fromOffset: value[2] } : null;
  } catch { return null; }
}

/** Empty selects the complete inventory, including links with no positive width. */
export function transferSelected(links: readonly TransferLink[], ids: readonly string[]): readonly TransferLink[] {
  if (!ids.length) return links;
  const wanted = new Set(ids);
  return links.filter(link => wanted.has(link.id));
}
export function transferPartRaw(link: TransferLink, measure: TransferMeasure): bigint | null {
  return measure === 'states' ? BigInt(link.stateCount) : link.victoryPoints;
}
export const transferPartHour = (link: TransferLink) => link.states.at(-1)?.dates.afterHour ?? 0;

// The product's checked i64 fold is ordered. Prefix extrema retain intermediate
// overflows when VP reads arrive out of order, without re-folding all past terms.
interface Fold { sum: bigint; min: bigint; max: bigint; missing: number }
const EMPTY: Fold = { sum: 0n, min: 0n, max: 0n, missing: 0 };
const merge = (a: Fold, b: Fold): Fold => ({ sum: a.sum + b.sum, min: a.min < a.sum + b.min ? a.min : a.sum + b.min,
  max: a.max > a.sum + b.max ? a.max : a.sum + b.max, missing: a.missing + b.missing });
const leaf = (value: bigint | null): Fold => value == null ? { ...EMPTY, missing: 1 }
  : { sum: value, min: value < 0n ? value : 0n, max: value > 0n ? value : 0n, missing: 0 };
const raw = (fold: Fold) => fold.missing || fold.min < -(1n << 63n) || fold.max >= (1n << 63n) ? null : fold.sum;

/** Sparse steps at supplying reads. Work is O(occurrences log occurrences), and
 * storage holds source changes, never countries times campaign hours. */
export function transferHistory(links: readonly TransferLink[], measure: TransferMeasure, first: number, last: number, from = -Infinity): StackRun[][] {
  return links.map(link => {
    const states = (link.nativeStates ?? link.states).filter(state => state.dates.afterHour >= from && state.dates.afterHour <= last);
    let capacity = 1;
    while (capacity < states.length) capacity *= 2;
    const tree: Fold[] = Array(capacity * 2).fill(EMPTY);
    const events: { hour: number; index: number; value: bigint | null }[] = [];
    states.forEach((state: TransferOccurrence, index) => {
      const knownAt = Math.max(state.dates.afterHour, state.knownAt ?? state.dates.afterHour);
      if (measure === 'states') events.push({ hour: knownAt, index, value: 1n });
      else {
        events.push({ hour: knownAt, index, value: null });
        if (state.victoryPoints != null && state.victoryPointsReadHour != null)
          events.push({ hour: Math.max(knownAt, state.victoryPointsReadHour), index, value: BigInt(state.victoryPoints) });
      }
    });
    events.sort((a, b) => a.hour - b.hour || a.index - b.index);
    const runs: StackRun[] = [];
    let at = first, value: bigint | null = 0n;
    const append = (end: number) => {
      if (end < at || value == null) return;
      const previous = runs.at(-1);
      if (previous && previous.lastHour + 1 === at && previous.value === Number(value)) previous.lastHour = end;
      else runs.push({ firstHour: at, lastHour: end, value: Number(value) });
    };
    for (let i = 0; i < events.length;) {
      const hour = events[i].hour;
      if (hour > last) break;
      if (hour > first) { append(hour - 1); at = hour; }
      do {
        const event = events[i++];
        let slot = capacity + event.index;
        tree[slot] = leaf(event.value);
        while (slot > 1) { slot = Math.floor(slot / 2); tree[slot] = merge(tree[slot * 2], tree[slot * 2 + 1]); }
      } while (i < events.length && events[i].hour === hour);
      value = raw(tree[1]);
    }
    append(last);
    return runs;
  });
}
