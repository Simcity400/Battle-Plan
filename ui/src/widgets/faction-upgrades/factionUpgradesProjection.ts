import { countryFromKey } from '../../dashboard/countryKey.ts';
import { countryIdentityIndex } from '../../dashboard/countryIdentity.ts';
import { validateCountryGroupReply } from '../../dashboard/groupsReply.ts';
import type { CardWidgetDeclaration } from '../../families/card/declaration.ts';
import type { CardItemRow, CardModel, CardProjection, CardRow } from '../../families/card/model.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { wholeBranchProof } from '../../families/timeline/helpers.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { CountryGroupReply, FactionGroup, FactionRulesReply, FactionSelectionRun, HourSpan } from '../../generated/dashboard';
import { identityName } from '../../families/table/identityCell.ts';

export interface FactionUpgradesOptions {
  readonly groups: CountryGroupReply | null;
  readonly subject: string | null;
  readonly branchFirst: number;
}

const COLUMNS = { figure: 132, figureAlign: 'right', lists: true } as const;
const entryName = (faction: string) => `faction:${faction}`;
const SINCE_UNRECORDED = 'It began during an unrecorded stretch, so when is not recorded';
const SINCE_BEFORE = 'Held when the recording began, so when it began is not recorded';

const covers = (span: { readonly firstHour: number; readonly lastHour: number }, hour: number) => span.firstHour <= hour && hour <= span.lastHour;

function factionAt(groups: CountryGroupReply, subject: string | null, hour: number): { readonly faction: FactionGroup | null; readonly reason: string } {
  if (groups.unobserved.factions.some(span => covers(span, hour))) return { faction: null, reason: 'Faction membership was not recorded at this date.' };
  if (!subject) return { faction: null, reason: 'Choose a country to see its faction’s rules.' };
  const index = countryIdentityIndex(groups.countryIdentities);
  const tags = new Set(typeof countryFromKey(subject) === 'number' ? index.aliasesAt(index.selected([subject]), hour) : [subject]);
  const faction = groups.factions.find(item => item.members.some(run => tags.has(run.tag) && covers(run, hour))) ?? null;
  return faction ? { faction, reason: '' } : { faction: null, reason: 'This country is in no faction at this date.' };
}

function factionsEver(groups: CountryGroupReply | undefined, subject: string | null): string[] {
  if (!groups || !subject) return [];
  const index = countryIdentityIndex(groups.countryIdentities);
  const ever = typeof countryFromKey(subject) === 'number'
    ? (hour: number) => new Set(index.aliasesAt(index.selected([subject]), hour))
    : () => new Set([subject]);
  return groups.factions.filter(item => item.members.some(run => ever(run.firstHour).has(run.tag) || ever(run.lastHour).has(run.tag))).map(item => item.id);
}

function since(run: FactionSelectionRun, gaps: readonly HourSpan[], branchFirst: number): Value<string> {
  if (run.firstHour <= branchFirst) return notRecorded(SINCE_BEFORE);
  if (run.startsSegment || gaps.some(gap => covers(gap, run.firstHour - 1))) return notRecorded(SINCE_UNRECORDED);
  return knownValue(`since ${formatDateOnly(run.firstHour)}`);
}

function runAtHour(runs: readonly FactionSelectionRun[], hour: number): FactionSelectionRun | null {
  return runs.find(run => covers(run, hour)) ?? null;
}

function row(key: string, run: FactionSelectionRun, subtitle: string | undefined, gaps: readonly HourSpan[], branchFirst: number): CardItemRow {
  const title: Value<string> = run.name ? knownValue(run.name) : notRecorded(run.reason ?? 'The name was not recorded');
  return {
    kind: 'item',
    key,
    title,
    ...(subtitle ? { subtitle } : {}),
    figure: { value: since(run, gaps, branchFirst), figure: 'Since' },
  };
}

function nextBoundary(reply: FactionRulesReply, hour: number): number {
  let next = Infinity;
  for (const lane of [...reply.rules, ...reply.programmes, ...reply.minimums, ...reply.members, ...reply.doctrines]) {
    for (const run of lane.runs) {
      if (run.firstHour > hour && run.firstHour < next) next = run.firstHour;
      if (run.lastHour + 1 > hour && run.lastHour + 1 < next) next = run.lastHour + 1;
    }
  }
  return next;
}

function upgradeRows(readView: BoundedReadView, reply: FactionRulesReply, hour: number, branchFirst: number): CardItemRow[] {
  const gaps = reply.unobserved;
  const groupName = (group: number) => reply.groups.find(item => item.group === group)?.name ?? 'Upgrade group not recorded';
  const out: CardItemRow[] = [];
  for (const lane of reply.programmes) {
    const run = runAtHour(lane.runs, hour);
    if (run) out.push(row(`programme:${lane.slot}`, run, 'Shared programme', gaps, branchFirst));
  }
  for (const lane of reply.minimums) {
    const run = runAtHour(lane.runs, hour);
    if (run) out.push(row(`minimum:${lane.group}`, run, `Faction minimum · ${groupName(lane.group)}`, gaps, branchFirst));
  }
  for (const lane of reply.members) {
    const run = runAtHour(lane.runs, hour);
    if (!run) continue;
    const name = lane.tag ? identityName(readView, 'country', lane.tag, hour) : 'Member not recorded';
    out.push(row(`member:${lane.key}`, run, `${name} · ${groupName(lane.group)}`, gaps, branchFirst));
  }
  for (const lane of reply.doctrines) {
    const run = runAtHour(lane.runs, hour);
    if (run) out.push(row(`doctrine:${lane.slot}`, run, 'Shared doctrine', gaps, branchFirst));
  }
  return out;
}

export const factionUpgradesProjection: CardProjection<FactionUpgradesOptions> = registerFamilyProjection({
  family: 'card' as const,
  kind: 'faction-upgrades',
  project(readView, options): CardModel {
    const hour = readView.hour;
    const blank = (empty: string, emptyKind: 'empty' | 'unavailable' = 'empty'): CardModel => ({ hour, rows: [], columns: COLUMNS, rowHeight: 36, empty, emptyKind });
    if (!options.groups) return { hour, rows: [], columns: COLUMNS, rowHeight: 36 };
    const pick = factionAt(options.groups, options.subject, hour);
    if (!pick.faction) return blank(pick.reason, pick.reason.includes('not recorded') ? 'unavailable' : 'empty');
    const raw = readView.panel<FactionRulesReply>(entryName(pick.faction.id));
    if (!raw) return { hour, rows: [], columns: COLUMNS, rowHeight: 36 };
    if (raw.unobserved.some(span => covers(span, hour))) return blank('This faction’s rules were not recorded at this date.', 'unavailable');
    const rules = raw.rules.flatMap(lane => {
      const run = runAtHour(lane.runs, hour);
      return run ? [row(`rule:${lane.slot}`, run, undefined, raw.unobserved, options.branchFirst)] : [];
    });
    const upgrades = upgradeRows(readView, raw, hour, options.branchFirst);
    const rows: CardRow[] = [
      { kind: 'section', key: '__rules', label: 'Rules', count: rules.length },
      ...rules,
      { kind: 'section', key: '__upgrades', label: 'Upgrades', count: upgrades.length },
      ...upgrades,
    ];
    return {
      hour,
      rows,
      columns: COLUMNS,
      rowHeight: 36,
      nextChangeAt: nextBoundary(raw, hour),
      ...(raw.warnings.length ? { warnings: raw.warnings } : {}),
    };
  },
}) as CardProjection<FactionUpgradesOptions>;

export const factionUpgradesDeclaration: CardWidgetDeclaration<FactionUpgradesOptions, CountryGroupReply> = {
  family: 'card',
  kind: 'faction-upgrades',
  label: 'Faction upgrades & rules',
  subject: 'faction rules',
  emptyCampaign: 'Load a campaign to see faction rules.',
  icon: 'history',
  countries: { mode: 'single', choose: 'Choose a country to see its faction’s rules.' },
  keyByBranch: true,
  aux: {
    product: 'country-groups',
    params: track => ({ branchId: track.branch.id }),
    validate: validateCountryGroupReply,
  },
  sources: ctx => {
    const subject = ctx.countrySubject ?? ctx.countryTag;
    const entries = factionsEver(ctx.aux, subject).map(faction => ({
      name: entryName(faction),
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour' as const,
        coverage: { mode: 'keyed' as const, keys: [faction] },
        product: 'faction-state' as const,
        followQuery: null,
        params: { view: 'rules', branchId: ctx.branch.id, faction, viewpoint: subject === null ? null : countryFromKey(subject) },
        buildSpanParams: (params: Readonly<Record<string, unknown>>) => params,
        provenInterval: wholeBranchProof,
      },
    }));
    return {
      owner: `faction-upgrades:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${subject ?? ''}`,
      entries,
      allowEmptyFollow: entries.length === 0,
    };
  },
  projection: factionUpgradesProjection,
  projectionOptions: ctx => ({
    groups: ctx.aux ?? null,
    subject: ctx.countrySubject ?? ctx.countryTag,
    branchFirst: ctx.branch.firstHour,
  }),
};
