import type { TooltipRow } from '../../charts/tooltip';
import type { FactoryGroupsReply, FactoryMixPart, FactoryPartValue, FactoryPartsReply } from '../../generated/dashboard';

export const COUNT_MODES = [
  { value: 'free', label: 'Available', hint: 'Built less damaged, consumer goods, trade, national projects, licences, market contracts and factories given away' },
  { value: 'built', label: 'Built', hint: 'The game’s Total: every factory counted, damaged ones included' },
  { value: 'topBar', label: 'Top bar', hint: 'The top bar’s group total: built less damaged and factories given away' },
] as const;
export type CountMode = typeof COUNT_MODES[number]['value'];

const FACTORY_GROUPS = [
  { value: 'military', bit: 1 },
  { value: 'naval', bit: 2 },
  { value: 'civilian', bit: 4 },
] as const;

const GROUP_NAME_NOT_RECORDED = 'Name not recorded';

export interface FactoryGroupPresentation {
  readonly key: string; readonly label: string; readonly missing: string | null; readonly colorIndex: number;
}

export const countMode = (value: unknown): CountMode => COUNT_MODES.find(mode => mode.value === value)?.value ?? 'free';

function groupMask(groups: readonly string[]): number {
  return FACTORY_GROUPS.reduce((sum, group) => sum | (groups.includes(group.value) ? group.bit : 0), 0) || 7;
}

export function factoryMeasure(mode: CountMode, groups: readonly string[]): string {
  return `${mode}${groupMask(groups)}`;
}

export function damagedMeasure(groups: readonly string[]): string {
  return `damaged${groupMask(groups)}`;
}

function factoryGroupIndex(key: string): number {
  const index = FACTORY_GROUPS.findIndex(held => held.value === key);
  return index >= 0 ? index : 0;
}

export function factoryGroupPresentation(key: string, names: FactoryGroupsReply | undefined): FactoryGroupPresentation {
  const colorIndex = factoryGroupIndex(key);
  const group = names?.groups.find(held => held.key === key);
  if (group?.buildings.length) {
    const missing = group.buildings.some(building => Boolean(building.name)) ? null : 'Building name not recorded';
    return { key, label: group.buildings.map(building => building.name || 'Building name unavailable').join(' · '), missing, colorIndex };
  }
  const missing = names?.missing ?? (group ? `No building in the recorded content has ${group.flag}` : 'Factory group name not recorded');
  return { key, label: GROUP_NAME_NOT_RECORDED, missing, colorIndex };
}

export function damagedGroupLabel(presentation: Pick<FactoryGroupPresentation, 'label'>): string {
  return presentation.label === GROUP_NAME_NOT_RECORDED ? `Damaged (${presentation.label})` : `Damaged ${presentation.label}`;
}

export function mixPartPresentation(part: Pick<FactoryMixPart, 'group' | 'name' | 'damaged'>): FactoryGroupPresentation {
  const colorIndex = factoryGroupIndex(part.group);
  if (part.name) return { key: part.group, label: part.name, missing: null, colorIndex };
  const missing = 'Factory group name not recorded';
  return { key: part.group, label: part.damaged ? damagedGroupLabel({ label: GROUP_NAME_NOT_RECORDED }) : GROUP_NAME_NOT_RECORDED, missing, colorIndex };
}

export const typeChoices = (names?: FactoryGroupsReply) => FACTORY_GROUPS.map(group => {
  const { label, missing } = factoryGroupPresentation(group.value, names);
  return { value: group.value, label, ...(missing !== null ? { hint: missing } : {}) };
});

export const chosenGroups = (groups: readonly string[]) => FACTORY_GROUPS.filter(group => !groups.length || groups.includes(group.value));

export function validateFactoryGroups(value: unknown): FactoryGroupsReply {
  const reply = value as Partial<FactoryGroupsReply> | null;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.groups) || !Array.isArray(reply.warnings)
    || !(reply.missing === null || typeof reply.missing === 'string')
    || !reply.groups.every(group => group && typeof group.key === 'string' && typeof group.flag === 'string' && Array.isArray(group.buildings)
      && group.buildings.every(building => building && typeof building.token === 'string' && (building.name === null || typeof building.name === 'string')))) {
    throw new Error('The factory groups reply is malformed.');
  }
  return reply as FactoryGroupsReply;
}

export function validateFactoryParts(value: unknown): FactoryPartsReply {
  const reply = value as Partial<FactoryPartsReply> | null;
  if (!reply || typeof reply !== 'object' || !Number.isSafeInteger(reply.hour) || typeof reply.tag !== 'string' || !Array.isArray(reply.parts)
    || !reply.parts.every(part => part && typeof part.group === 'string' && typeof part.part === 'string' && Number.isSafeInteger(part.value))
    || !Array.isArray(reply.unexplained) || !(reply.missing === null || typeof reply.missing === 'string') || !Array.isArray(reply.warnings)) {
    throw new Error('The factory parts reply is malformed.');
  }
  return reply as FactoryPartsReply;
}

const PART_LABELS: Readonly<Record<string, string>> = {
  total: 'Built', controlled: 'In controlled states', owned: 'In owned states', damaged: 'Damaged', fromSubjects: 'From subjects',
  toOverlord: 'To overlord', fromOthers: 'From other countries', toOthers: 'To other countries', tradeExports: 'Earned by trade',
  licenseExports: 'Earned by licences', consumerGoods: 'Consumer goods', contractPayments: 'Market contracts', specialProjects: 'National projects',
  licenseImports: 'Licence imports', tradeImports: 'Trade imports',
};

export function groupCount(parts: readonly FactoryPartValue[], group: string, mode: CountMode): number | null {
  const get = (part: string) => parts.find(held => held.group === group && held.part === part)?.value;
  const total = get('total');
  if (total === undefined) return null;
  if (mode === 'built') return total;
  const given = (get('toOverlord') ?? 0) + (get('toOthers') ?? 0);
  const shown = total - (get('damaged') ?? 0) - given;
  if (mode === 'topBar' || group !== 'civilian') return shown;
  return ['consumerGoods', 'contractPayments', 'specialProjects', 'licenseImports', 'tradeImports'].reduce((left, part) => left - (get(part) ?? 0), shown);
}

export function partRows(reply: FactoryPartsReply, groups: readonly string[], mode: CountMode, names: FactoryGroupsReply | undefined): TooltipRow[] {
  if (reply.missing !== null) return [{ label: 'Not recorded', value: reply.missing, tone: 'muted' }];
  return chosenGroups(groups).flatMap(group => {
    const { label, missing } = factoryGroupPresentation(group.value, names);
    const count = groupCount(reply.parts, group.value, mode);
    return [{ label, value: count === null ? 'Not recorded' : count.toLocaleString('en-US'), ...(missing !== null ? { detail: missing } : {}), strong: true, divider: true } satisfies TooltipRow,
      ...reply.parts.filter(part => part.group === group.value && part.value !== 0)
        .map(part => ({ label: PART_LABELS[part.part] ?? part.part, value: part.value.toLocaleString('en-US') })),
      ...(reply.unexplained.includes(group.value) ? [{ label: 'Unavailable', value: 'The game withholds more than these parts explain', tone: 'muted' as const }] : [])];
  });
}
