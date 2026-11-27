import { colour } from '../../charts/palette.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { contentKey } from '../../dashboard/options.ts';
import { artTooltipItem } from '../../families/art/adapters.ts';
import { unavailableArt } from '../../families/art/types.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { barsLabelText, countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsRowLabel, type BarsRowMedia, type CountryRowPresentation } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatShare, formatValueState, type ValueFormat } from '../../families/valueFormat.ts';
import type {
  FactionPoolDoctrine,
  FactionPoolKind,
  FactionPoolManpower,
  FactionPoolResearch,
  FactionPoolResearchMember,
  FactionPoolResource,
  FactionPoolsReply,
  FactionPoolStatus,
} from '../../generated/dashboard';

export interface FactionPooledStrengthOptions {
  readonly pool: FactionPoolKind;
  readonly surface: string;
}

const POOLS = new Set<string>(['resources', 'manpower', 'research', 'doctrine']);
const STATUSES = new Set<string>(['ok', 'noFaction', 'factionAbsent', 'membershipNotRecorded', 'poolLocked', 'poolNotRecorded']);
const NO_ICON = 'The current installation has no icon for this resource';

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const optText = (value: unknown) => value === null || typeof value === 'string';
const optSlot = (value: unknown) => value === null || (Number.isSafeInteger(value) && (value as number) >= 0);

function validResource(value: unknown): value is FactionPoolResource {
  return isObject(value)
    && Number.isSafeInteger(value.ordinal) && (value.ordinal as number) >= 0
    && optText(value.token) && optText(value.name) && optText(value.icon)
    && optText(value.amount) && optText(value.nameReason) && optText(value.iconReason) && optText(value.missing)
    && (value.amount !== null || value.missing !== null);
}

function validManpower(value: unknown): value is FactionPoolManpower {
  return isObject(value)
    && Number.isSafeInteger(value.ordinal) && (value.ordinal as number) >= 0
    && optSlot(value.slot) && optText(value.tag) && optText(value.countryReason)
    && optText(value.amount) && optText(value.missing)
    && (value.amount !== null || value.missing !== null);
}

function validResearchMember(value: unknown): value is FactionPoolResearchMember {
  return isObject(value) && optSlot(value.slot) && optText(value.tag) && optText(value.countryReason);
}

function validResearch(value: unknown): value is FactionPoolResearch | null {
  if (value === null) return true;
  return isObject(value)
    && optText(value.scientistBase) && optText(value.scientistBaseMissing)
    && Array.isArray(value.members) && value.members.every(validResearchMember)
    && optText(value.membersMissing);
}

function validDoctrine(value: unknown): value is FactionPoolDoctrine {
  return isObject(value)
    && Number.isSafeInteger(value.ordinal) && (value.ordinal as number) >= 0
    && optText(value.token) && optText(value.name) && optText(value.reason);
}

export function validateFactionPools(value: unknown): FactionPoolsReply {
  if (
    !isObject(value)
    || typeof value.pool !== 'string' || !POOLS.has(value.pool)
    || typeof value.status !== 'string' || !STATUSES.has(value.status)
    || !optText(value.reason)
    || !optText(value.faction)
    || !Number.isSafeInteger(value.decimals) || (value.decimals as number) < 0 || (value.decimals as number) > 9
    || !Array.isArray(value.resources) || !value.resources.every(validResource)
    || !Array.isArray(value.manpower) || !value.manpower.every(validManpower)
    || !optText(value.manpowerTotal)
    || !validResearch(value.research)
    || !Array.isArray(value.doctrines) || !value.doctrines.every(validDoctrine)
    || !Array.isArray(value.warnings) || !value.warnings.every(item => typeof item === 'string')
  ) {
    throw new Error('The faction pools reply is malformed.');
  }
  return value as unknown as FactionPoolsReply;
}

function fixedValue(raw: string | null, reason: string | null): Value<bigint> {
  if (raw === null) return notRecorded(reason ?? 'Not recorded at this hour');
  const parsed = BigInt(raw);
  return parsed === 0n ? zeroValue(0n) : knownValue(parsed);
}

function resourceMedia(readView: BoundedReadView, entry: FactionPoolResource): BarsRowMedia {
  const name = entry.name ?? entry.token ?? 'Resource';
  if (entry.icon === null) {
    return { kind: 'art', art: unavailableArt(entry.iconReason ?? NO_ICON), resolvedName: name };
  }
  return { kind: 'art', art: readView.art({ kind: 'url', url: entry.icon }), resolvedName: name };
}

function memberPresentation(
  readView: BoundedReadView,
  slot: number | null,
  tag: string | null,
  reason: string | null,
): CountryRowPresentation {
  const key = slot !== null ? countryKey(slot) : tag;
  if (key !== null) {
    return countryRowPresentation(readView, key);
  }
  const missingReason = reason ?? 'Country identity was not recorded at this hour';
  return {
    label: notRecorded(missingReason),
    media: { kind: 'flag', art: unavailableArt(missingReason) },
  };
}

function nonOkModel(base: { readonly form: 'ordered'; readonly hour: number; readonly showAxis: false }, reply: FactionPoolsReply): BarsModel {
  const warnings = reply.warnings;
  const status: FactionPoolStatus = reply.status;
  if (status === 'noFaction') {
    return { ...base, rows: [], empty: 'This country is in no faction at this date.', emptyKind: 'empty', warnings };
  }
  if (status === 'factionAbsent') {
    return { ...base, rows: [], empty: reply.reason ?? 'This faction did not exist at this time.', emptyKind: 'empty', warnings };
  }
  if (status === 'poolLocked') {
    return { ...base, rows: [], empty: reply.reason ?? 'This faction has not unlocked this pool at this time.', emptyKind: 'empty', warnings };
  }
  if (status === 'membershipNotRecorded') {
    return { ...base, rows: [], empty: reply.reason ?? 'Faction rosters were not recorded at this time.', emptyKind: 'unavailable', warnings };
  }
  return { ...base, rows: [], empty: reply.reason ?? 'This faction’s pools were not recorded at this time.', emptyKind: 'unavailable', warnings };
}

function projectResources(readView: BoundedReadView, reply: FactionPoolsReply, surface: string): BarsModel {
  const base = { form: 'ordered' as const, hour: readView.hour, showAxis: false as const };
  const count = reply.resources.length;
  const summaryText = `${count} ${count === 1 ? 'resource' : 'resources'}`;
  if (count === 0) {
    return { ...base, rows: [], summaryText, empty: 'This faction pools no resources yet.', emptyKind: 'empty', warnings: reply.warnings };
  }
  const format: ValueFormat = { decimals: reply.decimals, maxDecimals: 2 };
  const exactFormat: ValueFormat = { decimals: reply.decimals, maxDecimals: reply.decimals };
  const scale = 10 ** reply.decimals;
  const rows: BarsDataRow[] = reply.resources.map(entry => {
    const value = fixedValue(entry.amount, entry.missing);
    const numeric = entry.amount === null ? null : Number(BigInt(entry.amount)) / scale;
    const media = resourceMedia(readView, entry);
    const label: BarsRowLabel = entry.name !== null ? entry.name : notRecorded(entry.nameReason ?? 'Resource name not recorded');
    const formatted = value.kind === 'known' || value.kind === 'zero' ? formatValueState(value, format) : null;
    const exactFormatted = value.kind === 'known' || value.kind === 'zero' ? formatValueState(value, exactFormat) : null;
    const items: TooltipListItem[] = [
      value.kind === 'notRecorded'
        ? { label: 'Pooled', missingReason: value.reason }
        : { label: 'Pooled', value: exactFormatted && 'text' in exactFormatted ? exactFormatted.text : '' },
    ];
    if (entry.nameReason) items.push({ label: 'Name', missingReason: entry.nameReason });
    const iconNote = artTooltipItem('Icon', media.art);
    if (iconNote) items.push(iconNote);
    return {
      kind: 'data',
      key: `resource:${entry.ordinal}`,
      subject: String(entry.ordinal),
      label,
      media,
      color: colour(surface, entry.ordinal),
      value: { value, format, label: 'Pooled' },
      numeric,
      ...(formatted && 'text' in formatted ? {
        readout: { primary: formatted.text, ...(value.kind === 'zero' ? { tone: 'muted' as const } : {}) },
      } : {}),
      tooltip: {
        title: entry.name ?? 'Resource name not recorded',
        hour: readView.hour,
        items,
        ...(value.kind === 'notRecorded' ? { missingReason: value.reason } : {}),
      },
    };
  });
  return { ...base, rows, summaryText, warnings: reply.warnings };
}

function projectManpower(readView: BoundedReadView, reply: FactionPoolsReply): BarsModel {
  const base = { form: 'ordered' as const, hour: readView.hour, showAxis: false as const };
  if (reply.manpower.length === 0) {
    return { ...base, rows: [], summaryText: '0 manpower', empty: 'No member contributes manpower yet.', emptyKind: 'empty', warnings: reply.warnings };
  }
  const totalBig = reply.manpowerTotal !== null ? BigInt(reply.manpowerTotal) : null;
  const totalNum = totalBig !== null ? Number(totalBig) : null;
  const totalFormatted = totalBig !== null ? formatValueState(totalBig === 0n ? zeroValue(0n) : knownValue(totalBig), 'integer') : null;
  const summaryText = totalFormatted && 'text' in totalFormatted
    ? `${totalFormatted.text} manpower`
    : 'Total not recorded';
  const rows: BarsDataRow[] = reply.manpower.map(entry => {
    const pres = memberPresentation(readView, entry.slot, entry.tag, entry.countryReason);
    const value = fixedValue(entry.amount, entry.missing);
    const numeric = entry.amount === null ? null : Number(BigInt(entry.amount));
    const formatted = value.kind === 'known' || value.kind === 'zero' ? formatValueState(value, 'integer') : null;
    const items: TooltipListItem[] = [
      value.kind === 'notRecorded'
        ? { label: 'Manpower contributed', missingReason: value.reason }
        : { label: 'Manpower contributed', value: formatted && 'text' in formatted ? formatted.text : '' },
    ];
    if (numeric !== null && totalNum !== null && totalNum > 0) {
      items.push({ label: 'Share of the pool', value: formatShare(numeric / totalNum) });
    }
    if (entry.countryReason) items.push({ label: 'Member', missingReason: entry.countryReason });
    const flagNote = artTooltipItem('Flag', pres.media.art);
    if (flagNote) items.push(flagNote);
    return {
      kind: 'data',
      key: `manpower:${entry.ordinal}`,
      subject: entry.slot !== null ? countryKey(entry.slot) : (entry.tag ?? String(entry.ordinal)),
      label: pres.label,
      media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      value: { value, format: 'integer', label: 'Manpower contributed' },
      numeric,
      ...(formatted && 'text' in formatted ? {
        readout: { primary: formatted.text, ...(value.kind === 'zero' ? { tone: 'muted' as const } : {}) },
      } : {}),
      tooltip: {
        title: barsLabelText(pres.label, 'Country'),
        hour: readView.hour,
        items,
        ...(value.kind === 'notRecorded' ? { missingReason: value.reason } : {}),
      },
    };
  });
  return { ...base, rows, summaryText, warnings: reply.warnings };
}

function projectResearch(readView: BoundedReadView, reply: FactionPoolsReply): BarsModel {
  const base = { form: 'ordered' as const, hour: readView.hour, showAxis: false as const };
  const research = reply.research;
  if (!research) {
    return { ...base, rows: [], empty: 'Faction research sharing was not recorded at this hour.', emptyKind: 'unavailable', warnings: reply.warnings };
  }
  const format: ValueFormat = { decimals: reply.decimals, maxDecimals: 2 };
  const exactFormat: ValueFormat = { decimals: reply.decimals, maxDecimals: reply.decimals };
  const baseVal = fixedValue(research.scientistBase, research.scientistBaseMissing);
  const formatted = baseVal.kind === 'known' || baseVal.kind === 'zero' ? formatValueState(baseVal, format) : null;
  const exactFormatted = baseVal.kind === 'known' || baseVal.kind === 'zero' ? formatValueState(baseVal, exactFormat) : null;
  const rows: BarsDataRow[] = [
    {
      kind: 'data',
      key: 'research:base',
      label: 'Scientist base',
      value: { value: baseVal, format, label: 'Scientist base' },
      numeric: null,
      ...(formatted && 'text' in formatted ? { readout: { primary: formatted.text } } : {}),
      tooltip: {
        title: 'Scientist base',
        hour: readView.hour,
        items: [
          baseVal.kind === 'notRecorded'
            ? { label: 'Value', missingReason: baseVal.reason }
            : { label: 'Value', value: exactFormatted && 'text' in exactFormatted ? exactFormatted.text : '' },
        ],
        ...(baseVal.kind === 'notRecorded' ? { missingReason: baseVal.reason } : {}),
      },
    },
    ...research.members.map((member, index): BarsDataRow => {
      const pres = memberPresentation(readView, member.slot, member.tag, member.countryReason);
      const items: TooltipListItem[] = [{ label: 'Research', value: 'Shares research with the faction' }];
      if (member.countryReason) items.push({ label: 'Member', missingReason: member.countryReason });
      const flagNote = artTooltipItem('Flag', pres.media.art);
      if (flagNote) items.push(flagNote);
      return {
        kind: 'data',
        key: `research:member:${index}`,
        subject: member.slot !== null ? countryKey(member.slot) : (member.tag ?? String(index)),
        label: pres.label,
        media: pres.media,
        numeric: null,
        readout: { primary: 'Sharing', tone: 'muted' },
        tooltip: {
          title: barsLabelText(pres.label, 'Country'),
          hour: readView.hour,
          items,
        },
      };
    }),
  ];
  return { ...base, rows, summaryText: `${research.members.length} sharing`, warnings: reply.warnings };
}

function projectDoctrines(readView: BoundedReadView, reply: FactionPoolsReply): BarsModel {
  const base = { form: 'ordered' as const, hour: readView.hour, showAxis: false as const };
  const count = reply.doctrines.length;
  const summaryText = `${count} ${count === 1 ? 'doctrine' : 'doctrines'}`;
  if (count === 0) {
    return { ...base, rows: [], summaryText, empty: 'This faction shares no doctrines yet.', emptyKind: 'empty', warnings: reply.warnings };
  }
  const rows: BarsDataRow[] = reply.doctrines.map(folder => {
    const label: BarsRowLabel = folder.name !== null ? folder.name : notRecorded(folder.reason ?? 'Doctrine name not recorded');
    const items: TooltipListItem[] = [{ label: 'Doctrine', value: 'Shared with the faction' }];
    if (folder.reason) items.push({ label: 'Name', missingReason: folder.reason });
    return {
      kind: 'data',
      key: `doctrine:${folder.ordinal}`,
      label,
      numeric: null,
      readout: { primary: 'Shared', tone: 'muted' },
      tooltip: {
        title: folder.name ?? 'Doctrine name not recorded',
        hour: readView.hour,
        items,
      },
    };
  });
  return { ...base, rows, summaryText, warnings: reply.warnings };
}

function projectPool(readView: BoundedReadView, options: FactionPooledStrengthOptions): BarsModel {
  const base = { form: 'ordered' as const, hour: readView.hour, showAxis: false as const };
  const refused = readView.refused('main');
  if (refused) return { ...base, rows: [], empty: refused, emptyKind: 'unavailable' };
  const panel = readView.panel('main');
  if (panel === undefined) return { ...base, rows: [] };
  const reply = validateFactionPools(panel);
  if (reply.status !== 'ok') return nonOkModel(base, reply);
  if (options.pool === 'resources') return projectResources(readView, reply, options.surface);
  if (options.pool === 'manpower') return projectManpower(readView, reply);
  if (options.pool === 'research') return projectResearch(readView, reply);
  return projectDoctrines(readView, reply);
}

function poolSources(ctx: FamilyPlanContext, pool: FactionPoolKind): SourcePlan {
  const explicitFaction = typeof ctx.options.faction === 'string' && ctx.options.faction ? contentKey(ctx.options.faction) : null;
  const subject = ctx.countrySubject;
  const owner = `faction-pools:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${pool}:${explicitFaction ?? subject ?? ''}`;
  if (!explicitFaction && !subject) {
    return { owner, entries: [] };
  }
  const params = explicitFaction
    ? { branchId: ctx.branch.id, faction: explicitFaction, pool }
    : { branchId: ctx.branch.id, country: countryFromKey(subject!), pool };
  return {
    owner,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'all' },
        product: 'faction-pools',
        followQuery: { panel: 'faction-pools', ...params },
        params,
        validate: validateFactionPools,
      },
    }],
  };
}

function makePoolView(pool: FactionPoolKind): {
  readonly projection: BarsProjection<FactionPooledStrengthOptions>;
  readonly declaration: BarsWidgetDeclaration<FactionPooledStrengthOptions>;
} {
  const projection: BarsProjection<FactionPooledStrengthOptions> = registerFamilyProjection({
    kind: 'faction-pooled-strength',
    family: 'bars',
    nextChange: () => Infinity,
    project: projectPool,
  }, pool);
  const declaration: BarsWidgetDeclaration<FactionPooledStrengthOptions> = {
    family: 'bars',
    kind: 'faction-pooled-strength',
    label: 'Faction pooled strength',
    subject: 'faction pools',
    emptyCampaign: 'Load a campaign to see faction pools.',
    icon: 'layers',
    countries: { mode: 'single', choose: 'Choose a country to see its faction’s pools.', subjectOption: 'faction' },
    sources: ctx => poolSources(ctx, pool),
    projection,
    projectionOptions: (_ctx, extra) => ({ pool, surface: extra.surface }),
    jumpToDate: true,
    keyByBranch: true,
  };
  return { projection, declaration };
}

const resourcesView = makePoolView('resources');
const manpowerView = makePoolView('manpower');
const researchView = makePoolView('research');
const doctrineView = makePoolView('doctrine');

export const factionPooledResourcesProjection = resourcesView.projection;
export const factionPooledResourcesDeclaration = resourcesView.declaration;
export const factionPooledManpowerProjection = manpowerView.projection;
export const factionPooledManpowerDeclaration = manpowerView.declaration;
export const factionPooledResearchProjection = researchView.projection;
export const factionPooledResearchDeclaration = researchView.declaration;
export const factionPooledDoctrineProjection = doctrineView.projection;
export const factionPooledDoctrineDeclaration = doctrineView.declaration;
