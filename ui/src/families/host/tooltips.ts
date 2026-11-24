import { fitTooltip, hideTooltip, showTooltip, tooltipText, type TooltipContent, type TooltipRow } from '../../charts/tooltip.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { formatBoundDate, formatDateHover, type DateBoundKind } from '../valueFormat.ts';

export const HOVER_OPEN_MS = 150;
export const HOVER_CLOSE_MS = 100;

export interface TooltipDateBound {
  readonly kind: DateBoundKind;
  readonly hour: number;
  readonly reason: string;
}

export interface TooltipListItem {
  readonly label: string;
  readonly value?: string;
  readonly swatch?: string;
  readonly tone?: 'positive' | 'negative' | 'neutral' | 'muted';
  readonly indent?: boolean;
  readonly strong?: boolean;
  readonly divider?: boolean;
  readonly missingReason?: string;
}

export interface FamilyTooltipPayload {
  readonly title: string;
  readonly fullText?: string;
  readonly hour?: number;
  readonly subtitle?: string;
  readonly dateBound?: TooltipDateBound;
  readonly missingReason?: string;
  readonly exactValue?: string;
  readonly items?: readonly TooltipListItem[];
  readonly note?: string;
  readonly description?: string;
  readonly detail?: string;
  readonly scrollable?: boolean;
  readonly scrollId?: string;
}

export type FamilyActionKind = 'detail' | 'battle' | 'event' | 'game-window' | 'focus' | 'pin' | 'custom';

export interface FamilyActionPayload {
  readonly actionId: string;
  readonly kind: FamilyActionKind;
  readonly subject: string;
  readonly hour: number;
  readonly rowKey?: string;
  readonly columnKey?: string;
  readonly title?: string;
  readonly data?: Readonly<Record<string, string | number | boolean | null>>;
}

export function buildTooltipContent(payload: FamilyTooltipPayload): TooltipContent {
  const title = payload.fullText && payload.fullText.trim() ? payload.fullText : payload.title;
  const bound = payload.dateBound
    ? formatBoundDate(payload.dateBound.hour, payload.dateBound.kind, payload.dateBound.reason)
    : null;
  const subtitle = payload.subtitle
    ?? bound?.hover
    ?? (payload.hour !== undefined ? formatDateHover(payload.hour) : undefined);
  const rows: TooltipRow[] = [];
  if (payload.exactValue !== undefined) {
    rows.push({ label: 'Exact value', value: payload.exactValue, strong: true });
  }
  if (payload.missingReason) {
    rows.push({ label: 'Not recorded', valueState: { kind: 'missing', reason: payload.missingReason }, detail: payload.missingReason });
  }
  if (payload.items) {
    for (const item of payload.items) {
      rows.push({
        label: item.label,
        ...(item.missingReason
          ? { valueState: { kind: 'missing', reason: item.missingReason }, detail: item.missingReason }
          : item.value !== undefined ? { value: item.value } : {}),
        ...(item.swatch ? { swatch: item.swatch } : {}),
        ...(item.tone ? { tone: item.tone } : {}),
        ...(item.indent ? { indent: true } : {}),
        ...(item.strong ? { strong: true } : {}),
        ...(item.divider ? { divider: true } : {}),
      });
    }
  }
  const notes = [bound?.reason, payload.note].filter((text): text is string => Boolean(text && text.trim()));
  return {
    title,
    ...(subtitle ? { subtitle } : {}),
    rows,
    ...(notes.length ? { note: notes.join(' ') } : {}),
    ...(payload.description ? { description: payload.description } : {}),
    ...(payload.detail ? { detail: payload.detail } : {}),
    ...(payload.scrollable ? { scrollable: true } : {}),
    ...(payload.scrollId ? { scrollId: payload.scrollId } : {}),
  };
}

export type FamilyTooltipEntry =
  | { readonly hour: number; readonly rowKey: string; readonly content: TooltipContent; readonly payload?: never }
  | { readonly hour: number; readonly rowKey: string; readonly payload: FamilyTooltipPayload; readonly content?: never };

export function resolveTooltipEntryContent(entry: FamilyTooltipEntry): TooltipContent {
  return entry.content ?? buildTooltipContent(entry.payload);
}

export interface FamilyTooltipField {
  readonly key: string;
  readonly text: FamilyTooltipEntry | null;
}

interface FittedCache {
  raw: TooltipContent;
  subtitle: string;
  height: number;
  fitted: TooltipContent;
}

export interface PinnedTooltip {
  readonly key: string;
  readonly content: TooltipContent;
  readonly x: number;
  readonly y: number;
}

export interface TooltipSyncInput {
  readonly dim: boolean;
  readonly held: boolean;
  readonly inspectedKey: string | null;
  readonly inspectedToken?: string | null;
  readonly pointerPos: { readonly x: number; readonly y: number } | null;
  readonly hoverTip: FamilyTooltipEntry | null;
  readonly hoverFieldKey?: string | null;
  readonly keepHoverWhilePending?: boolean;
  readonly focusInside: boolean;
  readonly focusedKey: string | null;
  readonly focusAnchor: { readonly x: number; readonly y: number };
  readonly focusTip: FamilyTooltipEntry | null;
  readonly viewportHeight: number;
}

export class FamilyTooltipController {
  private readonly owner: string;
  private hoverCache: FittedCache | null = null;
  private focusCache: FittedCache | null = null;
  private pinned: PinnedTooltip | null = null;
  private hoverShown = false;
  private tipVisible = false;
  private hoverTimer: ReturnType<typeof setTimeout> | null = null;
  private leaveTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(owner: string) {
    this.owner = owner;
  }

  get tooltipOwner(): string {
    return this.owner;
  }

  isPinned(): boolean {
    return this.pinned !== null;
  }

  fitCached(channel: 'hover' | 'focus', raw: TooltipContent, hour: number, viewportHeight: number): TooltipContent {
    const subtitle = formatDateTime(Math.floor(hour));
    const cached = channel === 'hover' ? this.hoverCache : this.focusCache;
    if (cached && cached.raw === raw && cached.subtitle === subtitle && cached.height === viewportHeight) {
      return cached.fitted;
    }
    const dated = raw.subtitle ? raw : { ...raw, subtitle };
    const fitted = fitTooltip([dated], viewportHeight);
    const entry: FittedCache = { raw, subtitle, height: viewportHeight, fitted };
    if (channel === 'hover') this.hoverCache = entry;
    else this.focusCache = entry;
    return fitted;
  }

  describeFocused(tip: FamilyTooltipEntry, hour: number, viewportHeight: number): string {
    return tooltipText(this.fitCached('focus', resolveTooltipEntryContent(tip), hour, viewportHeight));
  }

  show(
    channel: 'hover' | 'focus',
    tip: FamilyTooltipEntry | null,
    pos: { readonly x: number; readonly y: number } | null,
    viewportHeight: number,
  ): void {
    if (this.pinned) return;
    if (!tip || !pos) {
      this.hoverShown = false;
      if (this.tipVisible) {
        this.tipVisible = false;
        hideTooltip(this.owner);
      }
      return;
    }
    this.hoverShown = true;
    this.tipVisible = true;
    const fitted = this.fitCached(channel, resolveTooltipEntryContent(tip), tip.hour, viewportHeight);
    showTooltip(this.owner, fitted, pos.x, pos.y, tip.rowKey);
  }

  isHoverShown(): boolean {
    return this.hoverShown;
  }

  syncFromInstall(input: TooltipSyncInput): void {
    if (!input.dim && !input.held) {
      if (input.inspectedKey && input.pointerPos) {
        if (input.hoverTip && input.hoverTip.rowKey === input.inspectedKey) {
          this.show('hover', input.hoverTip, input.pointerPos, input.viewportHeight);
        } else if (input.keepHoverWhilePending && (!input.inspectedToken || input.hoverFieldKey !== input.inspectedToken)) {
          return;
        } else {
          this.hideIfUnpinned();
        }
      } else if (input.focusInside && input.focusedKey && input.focusTip && input.focusTip.rowKey === input.focusedKey) {
        this.show('focus', input.focusTip, input.focusAnchor, input.viewportHeight);
      } else {
        this.hideIfUnpinned();
      }
    } else {
      this.hideIfUnpinned();
    }
  }

  cancelHoverOpen(): void {
    if (this.hoverTimer !== null) {
      clearTimeout(this.hoverTimer);
      this.hoverTimer = null;
    }
  }

  cancelHoverLeave(): void {
    if (this.leaveTimer !== null) {
      clearTimeout(this.leaveTimer);
      this.leaveTimer = null;
    }
  }

  cancelTimers(): void {
    this.cancelHoverOpen();
    this.cancelHoverLeave();
  }

  scheduleHoverOpen(
    read: () => {
      readonly tip: FamilyTooltipEntry | null;
      readonly pos: { readonly x: number; readonly y: number } | null;
      readonly viewportHeight: number;
      readonly blocked: boolean;
    },
  ): void {
    this.cancelHoverOpen();
    const delay = this.hoverShown ? 0 : HOVER_OPEN_MS;
    this.hoverTimer = setTimeout(() => {
      this.hoverTimer = null;
      const current = read();
      if (current.blocked) return;
      if (current.tip) this.show('hover', current.tip, current.pos, current.viewportHeight);
    }, delay);
  }

  scheduleHoverLeave(onLeave: () => void): void {
    this.cancelHoverOpen();
    this.cancelHoverLeave();
    this.leaveTimer = setTimeout(() => {
      this.leaveTimer = null;
      onLeave();
    }, HOVER_CLOSE_MS);
  }

  pinFocused(
    tip: FamilyTooltipEntry | null,
    hour: number,
    pos: { readonly x: number; readonly y: number },
    viewportHeight: number,
    announce?: (message: string) => void,
  ): boolean {
    if (!tip) return false;
    const dateText = formatDateTime(Math.floor(hour));
    const pinnedContent: TooltipContent = { ...resolveTooltipEntryContent(tip), subtitle: `Pinned at ${dateText}` };
    this.pinned = { key: tip.rowKey, content: pinnedContent, x: pos.x, y: pos.y };
    this.tipVisible = true;
    showTooltip(this.owner, fitTooltip([pinnedContent], viewportHeight), pos.x, pos.y, `${tip.rowKey}:pinned`);
    announce?.('Tooltip pinned.');
    return true;
  }

  unpin(announce?: (message: string) => void): boolean {
    if (!this.pinned) return false;
    this.pinned = null;
    this.tipVisible = false;
    hideTooltip(this.owner);
    announce?.('Tooltip unpinned.');
    return true;
  }

  hideIfUnpinned(): void {
    if (this.pinned || !this.tipVisible) return;
    this.hoverShown = false;
    this.tipVisible = false;
    hideTooltip(this.owner);
  }

  clearPinAndHide(): void {
    this.cancelTimers();
    this.pinned = null;
    this.hoverShown = false;
    this.tipVisible = false;
    hideTooltip(this.owner);
  }

  dispose(): void {
    this.clearPinAndHide();
  }
}
