import { contentTitle } from '../../dashboard/contentTitles.ts';
import { runAt } from '../../charts/series.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { contentKey, setToken } from '../../dashboard/options.ts';
import type { BandStep, TimelineLane, TimelineSpan } from '../../charts/timeline/layout';
import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { formatAmount, formatFixed } from '../../charts/valueAxis.ts';
import { gbNumber } from '../../charts/numberFormat.ts';
import { object } from '../../dashboard/validation.ts';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import type { EquipmentIcon, LineHistory, LineNow, LineStep, OrganisationName, OutputNow, OutputPart, ProductionLineBreakdownReply, ProductionLinesAtReply, ProductionLinesReply, ProductionModifierRun, VariantName } from '../../generated/dashboard';

const RAW = /^-?\d{1,20}$/;
const PART = /^-?\d+$/;
const ID = /^\d{1,20}$/;
const HOUR = (value: unknown): value is number => Number.isInteger(value);
const KINDS = ['military', 'naval', 'shipRefit', 'railwayGun'];
const TEXT = (value: unknown) => value === null || typeof value === 'string';
const ordered = <T extends { firstHour: number; lastHour: number }>(runs: readonly T[]) =>
  runs.every((run, index) => HOUR(run.firstHour) && HOUR(run.lastHour) && run.firstHour <= run.lastHour && (index === 0 || runs[index - 1].lastHour < run.firstHour));
const isPart = (part: OutputPart) => object(part) && ID.test(part.variant) && ID.test(part.mio) && RAW.test(part.units) && typeof part.exact === 'boolean';
const isOutput = (run: { parts: OutputPart[]; sinceHour: number; wholeLife: boolean } | null) => run === null
  || object(run) && Array.isArray(run.parts) && run.parts.every(isPart) && HOUR(run.sinceHour) && typeof run.wholeLife === 'boolean';
const isVariant = (variant: VariantName) => object(variant) && ID.test(variant.id) && (variant.hour === undefined || HOUR(variant.hour)) && typeof variant.known === 'boolean'
  && typeof variant.token === 'string' && [variant.name, variant.typeName, variant.nameUnknown, variant.icon, variant.iconUnknown].every(TEXT);
const isOrganisation = (organisation: OrganisationName) => object(organisation) && ID.test(organisation.id) && (organisation.hour === undefined || HOUR(organisation.hour))
  && typeof organisation.token === 'string' && [organisation.name, organisation.nameUnknown, organisation.icon, organisation.iconUnknown].every(TEXT);
const isIcon = (icon: EquipmentIcon) => object(icon) && typeof icon.key === 'string' && typeof icon.image === 'string';
const isStep = (step: LineStep) => typeof step.startsSegment === 'boolean' && ID.test(step.variant) && Number.isInteger(step.activeFactories) && Number.isInteger(step.requestedFactories);
const isLine = (line: LineHistory) => object(line) && typeof line.key === 'string' && ID.test(line.handle) && (line.kind === null || KINDS.includes(line.kind))
  && Array.isArray(line.steps) && ordered(line.steps) && line.steps.every(isStep) && isOutput(line.finalOutput)
  && (line.endedBy === null || HOUR(line.endedBy)) && typeof line.beforeRecording === 'boolean';

function invalid(what: string): never {
  throw new Error(`Invalid ${what} reply`);
}

export function validateProductionReply(data: unknown): ProductionLinesReply {
  const reply = data as ProductionLinesReply;
  if (!object(reply) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !Number.isInteger(reply.decimals)
    || !Array.isArray(reply.lines) || !reply.lines.every(isLine) || new Set(reply.lines.map(line => line.key)).size !== reply.lines.length
    || !Array.isArray(reply.variants) || !reply.variants.every(isVariant) || !Array.isArray(reply.organisations) || !reply.organisations.every(isOrganisation)
    || !Array.isArray(reply.icons) || !reply.icons.every(isIcon) || !Array.isArray(reply.lineHours) || !ordered(reply.lineHours)
    || !Number.isInteger(reply.detailHours) || reply.detailHours <= 0 || !Array.isArray(reply.warnings)) invalid('production lines');
  return reply;
}

const isOutputNow = (output: OutputNow | null) => output === null || object(output) && Array.isArray(output.parts)
  && output.parts.every(part => object(part) && ID.test(part.variant) && ID.test(part.mio) && typeof part.exact === 'boolean') && HOUR(output.sinceHour) && typeof output.wholeLife === 'boolean';
const isNow = (line: LineNow) => object(line) && ID.test(line.handle) && (line.kind === null || KINDS.includes(line.kind)) && ID.test(line.variant) && ID.test(line.mio)
  && [line.maxFactories, line.priority].every(Number.isInteger) && isOutputNow(line.output);

export function validateAtReply(data: unknown): ProductionLinesAtReply {
  const reply = data as ProductionLinesAtReply;
  if (!object(reply) || typeof reply.branchId !== 'string' || typeof reply.tag !== 'string' || !Array.isArray(reply.lines) || !reply.lines.every(isNow)
    || !TEXT(reply.missing) || !TEXT(reply.outputMissing) || !Array.isArray(reply.variants) || !reply.variants.every(isVariant)
    || !Array.isArray(reply.organisations) || !reply.organisations.every(isOrganisation) || !Array.isArray(reply.icons) || !reply.icons.every(isIcon)
    || !Array.isArray(reply.warnings)) invalid('production lines at an hour');
  return reply;
}

export function validateBreakdownReply(data: unknown): ProductionLineBreakdownReply {
  const reply = data as ProductionLineBreakdownReply;
  if (!object(reply) || typeof reply.key !== 'string' || !HOUR(reply.firstHour) || !HOUR(reply.lastHour) || !Array.isArray(reply.groups)
    || !Array.isArray(reply.runs) || !ordered(reply.runs) || !reply.runs.every(run => Array.isArray(run.groups) && run.groups.every(group =>
      typeof group.id === 'string' && TEXT(group.totalRaw) && Array.isArray(group.parts) && group.parts.every(part => typeof part.id === 'string' && PART.test(part.raw))))
    || !Array.isArray(reply.warnings)) invalid('production line breakdown');
  return reply;
}

export const productionLineToken = (campaign: string, branch: string, key: string) =>
  setToken(campaignToken(campaign, JSON.stringify([branch, key])));

export function productionLineKey(campaign: string, branch: string, token: unknown): string | null {
  if (typeof token !== 'string') return null;
  const value = contentKey(token);
  const prefix = campaignToken(campaign, '');
  if (!value.startsWith(prefix)) return null;
  try {
    const parts: unknown = JSON.parse(value.slice(prefix.length));
    const key = Array.isArray(parts) && parts[0] === branch ? parts.length === 2 ? parts[1] : parts.length === 3 && typeof parts[1] === 'string' ? parts[2] : null : null;
    return typeof key === 'string' && key !== '' ? key : null;
  } catch { return null; }
}

export const variantOf = (variants: readonly VariantName[], id: string): VariantName | null => variants.find(variant => variant.id === id && variant.known) ?? null;
export const organisationOf = (organisations: readonly OrganisationName[], id: string): OrganisationName | null =>
  id === '0' ? null : organisations.find(organisation => organisation.id === id) ?? null;

export const variantName = (variant: VariantName | null, id: string) => variant ? variant.name ?? variant.typeName ?? 'Variant name unavailable' : `Variant ${id}`;

export const organisationText = (organisation: OrganisationName | null, mio: string) =>
  mio === '0' ? '—' : organisation ? contentTitle(organisation.name, organisation.token, 'Organisation name unavailable') : 'Not recorded';

export const iconImage = (icons: readonly EquipmentIcon[], key: string | null): string | null =>
  key == null ? null : drawableIcon(icons.find(icon => icon.key === key)?.image);

export function weeklyText(weekly: bigint, decimals: number): string {
  const short = formatAmount(weekly, decimals, 1);
  return short === '0' && weekly !== 0n ? formatAmount(weekly, decimals, decimals) : short;
}

const EFFICIENCY_FULL = 3276800n;
export const efficiencyPercent = (raw: bigint) => (raw + 0x4000n) >> 15n;
export const efficiencyFraction = (raw: bigint) => Math.max(0, Math.min(1, Number(raw) / Number(EFFICIENCY_FULL)));

export function factoryScale(reply: ProductionLinesReply | undefined): number {
  let most = 0;
  for (const line of reply?.lines ?? []) for (const step of line.steps) most = Math.max(most, step.activeFactories, step.requestedFactories);
  return most;
}

export interface LineValues { active: number | null; requested: number | null; efficiency: bigint | null; speed: bigint | null; cost: bigint | null; output: number | null }

const whole = (value: number | undefined) => value === undefined ? null : BigInt(Math.round(value));

export function lineValues(values: LiveValues<number>, handle: string): LineValues {
  const read = (column: string) => values.get(liveKey(handle, column));
  return { active: read('active') ?? null, requested: read('requested') ?? null, efficiency: whole(read('efficiency')), speed: whole(read('speed')),
    cost: whole(read('cost')), output: read('output') ?? null };
}

export function weeklyRaw(speed: bigint | null, cost: bigint | null): bigint | null {
  return speed === null || cost === null || cost <= 0n ? null : speed * 700_000n / cost;
}

export function liveTotalText(output: OutputNow | null, units: number | null, missing: string | null): string {
  if (missing !== null || !output) return 'Not recorded';
  if (output.parts.some(part => !part.exact)) return 'Unknown';
  return units === null ? '—' : gbNumber.format(units);
}

export function outputTotal(parts: readonly OutputPart[]): bigint | null {
  if (parts.some(part => !part.exact)) return null;
  return parts.reduce((sum, part) => sum + BigInt(part.units), 0n);
}

export function totalText(output: { parts: readonly OutputPart[] } | null): string {
  if (!output) return 'Not recorded';
  const total = outputTotal(output.parts);
  return total == null ? 'Unknown' : gbNumber.format(total);
}

export function lineKeyAt(reply: ProductionLinesReply | undefined, handle: string, hour: number): string | null {
  const lines = reply?.lines.filter(line => line.handle === handle) ?? [];
  return lines.find(line => line.steps.length && line.steps[0].firstHour <= hour && hour <= line.steps.at(-1)!.lastHour)?.key ?? null;
}

export function productionLineLabel(reply: ProductionLinesReply, line: LineHistory, index: number, hour: number): string {
  const step = runAt(line.steps, hour) ?? line.steps.at(-1);
  const variant = step ? variantOf(reply.variants, step.variant) : null;
  return `${variant?.name ?? variant?.typeName ?? 'Equipment unavailable'} · line ${index + 1}`;
}

export const windowOf = (detailHours: number, hour: number) => Math.floor(hour / detailHours);

export function laneStatus(line: LineHistory, hour: number, day: (hour: number) => string): { text: string; ended: boolean } {
  const step = runAt(line.steps, hour);
  if (step) return { text: `${step.activeFactories} ${step.activeFactories === 1 ? 'factory' : 'factories'}`, ended: false };
  const first = line.steps[0], last = line.steps.at(-1);
  if (!first || !last) return { text: 'Not recorded then', ended: false };
  if (hour < first.firstHour) return { text: `Starts ${day(first.firstHour)}`, ended: false };
  if (line.endedBy != null && hour >= line.endedBy) {
    const total = line.finalOutput ? totalText(line.finalOutput) : null;
    return { text: total ? `Ended by ${day(line.endedBy)} · ${total} made` : `Ended by ${day(line.endedBy)}`, ended: true };
  }
  if (hour > last.lastHour) return { text: `Last recorded ${day(last.lastHour)}`, ended: true };
  return { text: 'Not recorded then', ended: false };
}

export function historyLanes(reply: ProductionLinesReply, until: number, showEnded: boolean, day: (hour: number) => string): TimelineLane[] {
  const lanes: TimelineLane[] = [];
  for (const line of reply.lines) {
    if (!line.steps.length || line.steps[0].firstHour > until) continue;
    const status = laneStatus(line, until, day);
    if (status.ended && !showEnded) continue;
    const spans: TimelineSpan[] = [];
    let band: BandStep[] = [];
    let first: LineStep | null = null;
    const close = (last: LineStep) => {
      if (!first) return;
      const variant = variantOf(reply.variants, first.variant);
      spans.push({ id: `${line.key}@${first.firstHour}`, firstHour: first.firstHour, lastHour: last.lastHour, label: variantName(variant, first.variant),
        icon: iconImage(reply.icons, variant?.icon ?? null) ?? undefined, emphasis: !status.ended, band });
      band = [];
      first = null;
    };
    line.steps.forEach((step, index) => {
      const previous = line.steps[index - 1];
      if (previous && (step.startsSegment || previous.variant !== step.variant || previous.lastHour + 1 !== step.firstHour)) close(previous);
      first ??= step;
      band.push({ firstHour: step.firstHour, lastHour: step.lastHour, value: step.activeFactories });
    });
    close(line.steps.at(-1)!);
    const shown = runAt(line.steps, until) ?? line.steps.filter(step => step.firstHour <= until).at(-1)!;
    const variant = variantOf(reply.variants, shown.variant);
    lanes.push({ id: line.key, label: status.text, spans, muted: status.ended,
      identity: { label: variantName(variant, shown.variant), icon: iconImage(reply.icons, variant?.icon ?? null) ?? undefined } });
  }
  return lanes;
}

export function bandScale(reply: ProductionLinesReply): number {
  let most = 0;
  for (const line of reply.lines) for (const step of line.steps) most = Math.max(most, step.activeFactories);
  return most;
}

export function modifierTooltip(group: ProductionModifierRun['groups'][number] | undefined, title: string, color: (id: string) => string | undefined): readonly TooltipContent[] {
  if (!group) return [{ title, rows: [{ label: 'Breakdown', value: 'Not recorded', tone: 'muted' }] }];
  const total: TooltipRow = { label: group.label, value: group.totalRaw == null ? 'Not recorded' : `${formatFixed(group.totalRaw, group.decimals, group.decimals)} ${group.unit}`, strong: true };
  const rows: TooltipRow[] = group.parts.map(part => ({ label: part.label ?? 'Other', value: `${formatFixed(part.raw, group.decimals, group.decimals)} ${group.unit}`,
    swatch: color(part.id), tone: part.unknown == null ? undefined : 'muted' }));
  return [{ title, rows: [total, ...rows] }, { title, rows: [total, { label: `${group.parts.length} factors`, tone: 'muted' }] }];
}
