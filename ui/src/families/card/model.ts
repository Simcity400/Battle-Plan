import { artTooltipItem } from '../art/adapters.ts';
import type { StatusMarkKind, ThresholdTick } from '../../draw/kit.ts';
import { flagChoiceArt, productIconArt } from '../art/provider.ts';
import type { ArtResult } from '../art/types.ts';
import { canonicalAuditValue } from '../audit.ts';
import { auditCell, type FamilyAuditRow } from '../auditRow.ts';
import type { BoundedReadView, FamilyProjection, MirrorModel, MirrorRow } from '../contract.ts';
import type { FamilyTooltipPayload, TooltipListItem } from '../host/tooltips.ts';
import { assertValue, type Value } from '../value.ts';
import { LOADING_NAME, resolveName } from '../table/identityCell.ts';
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
  readonly titleLoading?: true;
  readonly subtitle?: string;
  readonly subtitleFlag?: CardMedia;
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

export interface CardShareSegment {
  readonly share: number;
  readonly tone: 'positive' | 'negative';
  readonly label: string;
  readonly shortLabel?: string;
  readonly labelAlign?: 'left' | 'right';
}

export type CardBar =
  | { readonly kind: 'meter'; readonly fraction: Value<number>; readonly ticks?: readonly ThresholdTick[] }
  | { readonly kind: 'share'; readonly segments: readonly CardShareSegment[] };

export interface CardMeterRow {
  readonly kind: 'meter';
  readonly key: string;
  readonly label?: string;
  readonly name?: string;
  readonly bar: CardBar;
  readonly figure?: CardFigure;
  readonly tooltip?: FamilyTooltipPayload;
}

export interface CardListRow {
  readonly kind: 'list';
  readonly key: string;
  readonly label?: string;
  readonly name?: string;
  readonly items: readonly string[];
  readonly missing?: string;
  readonly none: string;
  readonly tooltip?: FamilyTooltipPayload;
}

export type CardRow = CardItemRow | CardSectionRow | CardHeroRow | CardMeterRow | CardListRow;
type CardEntryRow = Exclude<CardRow, CardSectionRow>;

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
    if (row.kind === 'meter') {
      if (row.bar.kind === 'meter') assertValue(row.bar.fraction, `card[${row.key}].bar`);
      if (row.figure) assertValue(row.figure.value, `card[${row.key}].figure`);
      continue;
    }
    if (row.kind === 'list') continue;
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

export function cardFlag(readView: BoundedReadView, tag: string, atHour?: number): { readonly media: CardMedia; readonly name: Value<string>; readonly loading: boolean } {
  const { value, loading, who } = resolveName(readView, 'country', tag, atHour === undefined ? {} : { atHour });
  const art: ArtResult = flagChoiceArt(readView, who.flagChoice);
  return { media: { kind: 'flag', art, tag }, name: value, loading };
}

export function cardTitleText(title: Value<string>, figure = 'Name'): string {
  return title.kind === 'known' || title.kind === 'partial' ? String(title.value) : notRecordedLabel(figure);
}

export function cardTitle(row: CardItemRow | CardHeroRow): string {
  return row.kind === 'item' && row.titleLoading ? LOADING_NAME : cardTitleText(row.title);
}

export function cardRowName(row: CardEntryRow): string {
  return row.kind === 'meter' || row.kind === 'list' ? row.name ?? row.label ?? row.key : cardTitle(row);
}

export function cardListText(row: CardListRow): string {
  if (row.missing !== undefined) return notRecordedLabel(row.label ?? row.name);
  return row.items.length ? row.items.join(', ') : row.none;
}

function figureText(figure: CardFigure): string {
  const formatted = formatValueState(figure.value);
  if (formatted.kind === 'notApplicable') return formatted.label;
  return formatted.kind === 'notRecorded' ? notRecordedLabel(figure.figure) : formatted.text;
}

function figureSpoken(figure: CardFigure): string {
  const formatted = formatValueState(figure.value);
  return formatted.kind === 'partial' || formatted.kind === 'notApplicable' ? formatted.spoken : figureText(figure);
}

function partialItem(value: Value | undefined): TooltipListItem[] {
  return value?.kind === 'partial' ? [{ label: value.reason, tone: 'muted' }] : [];
}

function artAudit(art: ArtResult): string {
  switch (art.status) {
    case 'ready': return `ready:${art.assetId}`;
    case 'noArtExpected':
    case 'pending': return art.status;
    default: return `${art.status}:${art.reason}`;
  }
}

function rowByKey(model: CardModel, key: string): CardEntryRow | null {
  for (const row of model.rows) if (row.kind !== 'section' && row.key === key) return row;
  return null;
}

export function deriveCardTooltip(model: CardModel, key: string): FamilyTooltipPayload | null {
  const row = rowByKey(model, key);
  if (!row) return null;
  if (row.tooltip) return row.tooltip.hour !== undefined ? row.tooltip : { ...row.tooltip, hour: model.hour };
  const title = cardRowName(row);
  const items: TooltipListItem[] = [];
  if (row.kind === 'meter' || row.kind === 'list') {
    const text = row.kind === 'list' ? cardListText(row) : row.figure ? figureText(row.figure) : row.bar.kind === 'share' ? row.bar.segments.map(s => s.label).join(' · ') : '';
    const value = row.kind === 'meter' ? row.figure?.value : undefined;
    const missing = row.kind === 'list' ? row.missing : value && (value.kind === 'notRecorded' || value.kind === 'notApplicable') ? value.reason : undefined;
    if (missing !== undefined) items.push({ label: row.label ?? 'Value', missingReason: missing });
    else if (text) items.push({ label: row.label ?? 'Value', value: text });
    items.push(...partialItem(value));
    return { title, fullText: title, hour: model.hour, ...(items.length ? { items } : {}) };
  }
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
        : { label: row.figure.figure ?? 'Value', value: formatted.text }, ...partialItem(row.figure.value));
    }
    if (row.state) items.push({ label: 'State', value: row.state.label });
  }
  for (const [label, media] of [['Flag', row.kind === 'item' ? row.flag : undefined], ['Art', row.art]] as const) {
    const item = artTooltipItem(label, media?.art);
    if (item) items.push(item);
  }
  return { title, fullText: title, hour: model.hour, ...(items.length ? { items } : {}) };
}

function spokenRow(row: CardEntryRow): string[] {
  const parts: string[] = [];
  if (row.kind === 'list') return [cardListText(row)];
  if (row.kind === 'meter') {
    if (row.bar.kind === 'share') parts.push(...row.bar.segments.map(s => s.label));
    if (row.figure) parts.push(figureSpoken(row.figure));
    return parts;
  }
  if (row.kind === 'hero') {
    parts.push(row.state.label);
    if (row.detail) parts.push(row.detail);
    if (row.figure) parts.push(`${row.figure.caption}: ${figureSpoken({ value: row.figure.value })}`);
    return parts;
  }
  if (row.date) parts.push(row.date.hour === null ? notRecordedLabel('Date') : formatDateOnly(row.date.hour));
  if (row.subtitleFlag?.name) parts.push(row.subtitleFlag.name);
  if (row.subtitle) parts.push(row.subtitle);
  if (row.place) parts.push(row.place);
  if (row.meter?.text) parts.push(row.meter.text);
  if (row.figure) parts.push(figureSpoken(row.figure));
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
    rows.push({ key: row.key, name: cardRowName(row), rowIndex: index, cells: spokenRow(row).map((spoken, at) => ({ column: String(at), spoken })) });
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
    if (row.kind === 'meter') {
      rows.push(auditCell(row.key, 'name', cardRowName(row)));
      rows.push(auditCell(row.key, 'bar', row.bar.kind === 'meter' ? canonicalAuditValue(row.bar.fraction) : row.bar.segments.map(s => `${s.tone}:${s.share}:${s.label}`).join('|')));
      if (row.figure) rows.push(auditCell(row.key, 'figure', canonicalAuditValue(row.figure.value)));
      continue;
    }
    if (row.kind === 'list') {
      rows.push(auditCell(row.key, 'name', cardRowName(row)));
      rows.push(auditCell(row.key, 'items', row.missing !== undefined ? `notRecorded:${row.missing}` : row.items.length ? row.items.join('|') : `none:${row.none}`));
      continue;
    }
    rows.push(auditCell(row.key, 'title', canonicalAuditValue(row.title)));
    if (row.kind === 'item' && row.titleLoading) rows.push(auditCell(row.key, 'loading', 'title'));
    const media = row.kind === 'item' ? [['flag', row.flag], ['subtitleFlag', row.subtitleFlag], ['art', row.art]] as const : [['art', row.art]] as const;
    for (const [cell, held] of media) if (held) rows.push(auditCell(row.key, cell, `${held.kind}:${held.tag ?? ''}:${held.name ?? ''}:${artAudit(held.art)}`));
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
