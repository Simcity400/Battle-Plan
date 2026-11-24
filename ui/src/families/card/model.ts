import { artTooltipItem } from '../art/adapters.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import { flagChoiceArt, productIconArt } from '../art/provider.ts';

import { canonicalAuditValue } from '../audit.ts';
import { auditCell, type FamilyAuditRow } from '../auditRow.ts';
import type { BoundedReadView, FamilyProjection, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyTooltipPayload, TooltipListItem } from '../host/tooltips.ts';
import { assertValue, type Value } from '../value.ts';
import { formatDateHover, formatDateOnly, formatValueState, notRecordedLabel } from '../valueFormat.ts';

export interface CardMedia {
  readonly kind: 'flag' | 'art';
  readonly art: ArtResult;
  readonly tag?: string;
  readonly name?: string;
}

export interface CardState {
  readonly mark: StatusMarkKind;
  readonly label: string;
}

export interface CardMeter {
  readonly fraction: Value<number>;
  readonly text?: string;
}

export interface CardFigure {
  readonly value: Value<string>;
  readonly figure?: string;
  readonly muted?: boolean;
}

export interface CardItemRow {
  readonly kind: 'item';
  readonly key: string;
  readonly subject?: string;
  readonly date?: { readonly hour: number | null; readonly reason?: string };
  readonly flag?: CardMedia;
  readonly art?: CardMedia;
  readonly title: Value<string>;
  readonly subtitle?: string;
  readonly place?: string;
  readonly meter?: CardMeter;
  readonly figure?: CardFigure;
  readonly state?: CardState;
  readonly muted?: boolean;
  readonly tooltip?: FamilyTooltipPayload;
}

export interface CardSectionRow {
  readonly kind: 'section';
  readonly key: string;
  readonly label: string;
  readonly count?: number;
}

export interface CardHeroRow {
  readonly kind: 'hero';
  readonly key: string;
  readonly subject?: string;
  readonly art?: CardMedia;
  readonly title: Value<string>;
  readonly state: CardState;
  readonly detail?: string;
  readonly figure?: { readonly value: Value<string>; readonly caption: string };
  readonly meter?: CardMeter;
  readonly tooltip?: FamilyTooltipPayload;
}

export type CardRow = CardItemRow | CardSectionRow | CardHeroRow;

export interface CardColumns {
  readonly date?: boolean;
  readonly flag?: boolean;
  readonly art?: { readonly width: number; readonly height: number };
  readonly titleWidth?: number;
  readonly titleWeight?: number;
  readonly place?: boolean;
  readonly meter?: number;
  readonly meterText?: number;
  readonly figure?: number;
  readonly figureAlign?: 'left' | 'right';
  readonly state?: number;
  readonly lists?: boolean;
}

export interface CardModel {
  readonly hour: number;
  readonly rows: readonly CardRow[];
  readonly columns: CardColumns;
  readonly rowHeight: number;
  readonly summaryText?: string;
  readonly empty?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly notes?: readonly string[];
  readonly warnings?: readonly string[];
  readonly nextChangeAt?: number;
}

export interface CardProjection<TOptions = unknown> extends FamilyProjection<CardModel, TOptions> {
  readonly family: 'card';
  project(readView: BoundedReadView, options: TOptions): CardModel;
  nextChange?(readView: BoundedReadView, options: TOptions): number;
}

export class FutureCardDataError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'FutureCardDataError';
  }
}

export function assertCardModelValid(model: CardModel, displayedHour: number): void {
  if (!Number.isFinite(model.hour) || model.hour > displayedHour) {
    throw new FutureCardDataError(`Card model hour ${model.hour} exceeds displayed hour ${displayedHour}`);
  }
  for (const row of model.rows) {
    if (row.kind === 'section') continue;
    assertValue(row.title, `card[${row.key}].title`);
    if (row.meter) assertValue(row.meter.fraction, `card[${row.key}].meter`);
    if (row.kind === 'item') {
      if (row.figure) assertValue(row.figure.value, `card[${row.key}].figure`);
      if (row.date && row.date.hour !== null && row.date.hour > displayedHour) {
        throw new FutureCardDataError(`card[${row.key}] is dated ${row.date.hour}, after displayed hour ${displayedHour}`);
      }
    } else if (row.figure) {
      assertValue(row.figure.value, `card[${row.key}].figure`);
    }
  }
}

export function cardRowHours(model: CardModel): Readonly<Record<string, number>> {
  const hours: Record<string, number> = {};
  for (const row of model.rows) if (row.kind === 'item' && row.date?.hour != null) hours[row.key] = row.date.hour;
  return hours;
}

export function cardMediaFromIcon(readView: BoundedReadView, url: string | null | undefined, missing: string | null | undefined, name?: string): CardMedia {
  return { kind: 'art', art: productIconArt(readView, url, missing), ...(name ? { name } : {}) };
}

export function cardFlag(readView: BoundedReadView, tag: string, atHour?: number): { readonly media: CardMedia; readonly name: Value<string> } {
  const identity = readView.who('country', tag, atHour);
  const name: Value<string> = identity.name.status === 'ready'
    ? { kind: 'known', value: identity.name.value }
    : identity.name.status === 'pending'
      ? { kind: 'known', value: tag }
      : { kind: 'notRecorded', reason: identity.name.reason };
  const art: ArtResult = flagChoiceArt(readView, identity.flagChoice);
  return { media: { kind: 'flag', art, tag }, name };
}

export function cardTitleText(title: Value<string>, figure = 'Name'): string {
  return title.kind === 'known' || title.kind === 'partial' ? String(title.value) : notRecordedLabel(figure);
}

function figureText(figure: CardFigure): string {
  const formatted = formatValueState(figure.value);
  return formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable' ? notRecordedLabel(figure.figure) : formatted.text;
}

function rowByKey(model: CardModel, key: string): CardItemRow | CardHeroRow | null {
  for (const row of model.rows) if (row.kind !== 'section' && row.key === key) return row;
  return null;
}

export function deriveCardTooltip(model: CardModel, key: string): FamilyTooltipPayload | null {
  const row = rowByKey(model, key);
  if (!row) return null;
  if (row.tooltip) return row.tooltip.hour !== undefined ? row.tooltip : { ...row.tooltip, hour: model.hour };
  const title = cardTitleText(row.title);
  const items: TooltipListItem[] = [];
  if (row.title.kind === 'notRecorded') items.push({ label: 'Name', missingReason: row.title.reason });
  if (row.kind === 'item') {
    if (row.date) {
      items.push(row.date.hour === null
        ? { label: 'Date', missingReason: row.date.reason ?? notRecordedLabel('Date') }
        : { label: 'Date', value: formatDateHover(row.date.hour) });
    }
    if (row.place) items.push({ label: 'Place', value: row.place });
    if (row.figure) {
      const formatted = formatValueState(row.figure.value);
      items.push(formatted.kind === 'notRecorded' || formatted.kind === 'notApplicable'
        ? { label: row.figure.figure ?? 'Value', missingReason: formatted.reason }
        : { label: row.figure.figure ?? 'Value', value: formatted.text });
    }
    if (row.state) items.push({ label: 'State', value: row.state.label });
  }
  for (const [label, media] of [['Flag', row.kind === 'item' ? row.flag : undefined], ['Art', row.art]] as const) {
    const item = artTooltipItem(label, media?.art);
    if (item) items.push(item);
  }
  return { title, fullText: title, hour: model.hour, ...(items.length ? { items } : {}) };
}

function spokenRow(row: CardItemRow | CardHeroRow): string[] {
  const parts: string[] = [];
  if (row.kind === 'hero') {
    parts.push(row.state.label);
    if (row.detail) parts.push(row.detail);
    if (row.figure) parts.push(`${figureText({ value: row.figure.value })} ${row.figure.caption}`);
    return parts;
  }
  if (row.date) parts.push(row.date.hour === null ? notRecordedLabel('Date') : formatDateOnly(row.date.hour));
  if (row.subtitle) parts.push(row.subtitle);
  if (row.place) parts.push(row.place);
  if (row.meter?.text) parts.push(row.meter.text);
  if (row.figure) parts.push(figureText(row.figure));
  if (row.state) parts.push(row.state.label);
  return parts;
}

export function deriveCardMirror(model: CardModel): MirrorModel {
  const rows: MirrorRow[] = [];
  model.rows.forEach((row, index) => {
    if (row.kind === 'section') {
      rows.push({ key: row.key, name: row.count !== undefined ? `${row.label} ${row.count}` : row.label, rowIndex: index, groupHeader: true, cells: [] });
      return;
    }
    rows.push({ key: row.key, name: cardTitleText(row.title), rowIndex: index, cells: spokenRow(row).map((spoken, at) => ({ column: String(at), spoken })) });
  });
  return { first: 0, total: model.rows.length, rows };
}

export function deriveCardAudit(model: CardModel): FamilyAuditRow[] {
  const rows: FamilyAuditRow[] = [];
  if (model.empty !== undefined && model.rows.length === 0) {
    rows.push(auditCell('empty', model.emptyKind ?? 'empty', model.empty));
    return rows;
  }
  for (const row of model.rows) {
    if (row.kind === 'section') {
      rows.push(auditCell(row.key, 'section', `${row.label}:${row.count ?? ''}`));
      continue;
    }
    rows.push(auditCell(row.key, 'title', canonicalAuditValue(row.title)));
    if (row.meter) {
      rows.push(auditCell(row.key, 'meter', canonicalAuditValue(row.meter.fraction)));
      if (row.meter.text !== undefined) rows.push(auditCell(row.key, 'meterText', row.meter.text));
    }
    if (row.kind === 'hero') {
      rows.push(auditCell(row.key, 'state', `${row.state.mark}:${row.state.label}`));
      if (row.detail !== undefined) rows.push(auditCell(row.key, 'detail', row.detail));
      if (row.figure) {
        rows.push(auditCell(row.key, 'figure', canonicalAuditValue(row.figure.value)));
        rows.push(auditCell(row.key, 'caption', row.figure.caption));
      }
      continue;
    }
    if (row.subtitle !== undefined) rows.push(auditCell(row.key, 'subtitle', row.subtitle));
    if (row.date) rows.push(auditCell(row.key, 'date', row.date.hour === null ? `notRecorded:${row.date.reason ?? ''}` : String(row.date.hour)));
    if (row.place) rows.push(auditCell(row.key, 'place', row.place));
    if (row.figure) rows.push(auditCell(row.key, 'figure', `${canonicalAuditValue(row.figure.value)}${row.figure.figure ? `:${row.figure.figure}` : ''}${row.figure.muted ? ':muted' : ''}`));
    if (row.state) rows.push(auditCell(row.key, 'state', `${row.state.mark}:${row.state.label}`));
    if (row.muted) rows.push(auditCell(row.key, 'muted', 'true'));
  }
  return rows;
}
