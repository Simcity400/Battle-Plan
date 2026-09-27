/** One chart tooltip for the whole dashboard. Charts publish structured content,
 * not HTML, so every tooltip looks the same and no recorded text is ever parsed
 * as markup. The layer renders it above every widget. */

export interface TooltipRow {
  label: string;
  value?: string;
  /** A colour square identifying the series. */
  swatch?: string;
  tone?: 'positive' | 'negative' | 'muted';
  /** Starts a new group with a divider above it. */
  divider?: boolean;
  /** A section heading or a total: the label in full text colour, bold. */
  strong?: boolean;
  /** Keeps the label on one line, shortened, where a long list must fit the window. */
  short?: boolean;
}
export interface TooltipContent { title: string; rows: readonly TooltipRow[]; note?: string }
export interface TooltipState { owner: string; content: TooltipContent; x: number; y: number }

type Listener = () => void;
let state: TooltipState | null = null;
let key = '';
const listeners = new Set<Listener>();

const emit = () => { for (const listener of listeners) listener(); };

/** Shows content at a client position. Re-publishing identical content only moves it.
 * `contentKey` identifies the content cheaply (such as its hour) so it need not be
 * serialised every frame; without it the content itself is compared. */
export function showTooltip(owner: string, content: TooltipContent, x: number, y: number, contentKey?: string): void {
  const nextKey = `${owner}\u0000${contentKey ?? JSON.stringify(content)}`;
  if (nextKey === key && state && state.x === x && state.y === y && (contentKey == null || state.content === content)) return;
  key = nextKey;
  state = { owner, content, x, y };
  emit();
}

/** Hides the tooltip if this owner is showing it. */
export function hideTooltip(owner: string): void {
  if (state?.owner !== owner) return;
  state = null;
  key = '';
  emit();
}

export const tooltipStore = {
  get: () => state,
  subscribe(listener: Listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
};
