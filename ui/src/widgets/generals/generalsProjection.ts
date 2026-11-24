import { contentTitle } from '../../dashboard/contentTitles.ts';
import { spriteIcons, spriteId, type SpriteFrame } from '../../dashboard/spriteArtwork.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  resolveArtUrlFromReadView,
  resolveCountryFromReadView,
  type TableCell,
  type TableCellIconItem,
  type TableColumnSpec,
  type TableModel,
  type TableProjection,
  type TableRow,
  type TooltipExtraRow,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY, type TableBase } from '../../families/table/settings.ts';
import { knownValue, notRecorded, partialValue, zeroValue } from '../../families/value.ts';
import type { LeaderKinds, UnitLeadersReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS } from './definition.ts';
import {
  attributeNames,
  leaderRequestSelection,
  leaderRows,
  traitDateText,
  validateLeaders,
  withheldLeaderState,
  withheldLeaders,
} from './generalsData.ts';

const SKILL_SPRITES: readonly SpriteFrame[] = [
  { key: 'GFX_land_battle_leader_skill_bg', frame: 0 },
  { key: 'GFX_trait_attack', frame: 0 },
  { key: 'GFX_trait_defense', frame: 0 },
  { key: 'GFX_trait_planning', frame: 0 },
  { key: 'GFX_trait_logistics', frame: 0 },
];

export interface LeaderSkillIcons {
  readonly level?: string;
  readonly attack?: string;
  readonly defense?: string;
  readonly planning?: string;
  readonly logistics?: string;
}

export interface GeneralsProjectionOptions {
  readonly kind: LeaderKinds;
  readonly filterAll: boolean;
  readonly filter: FamilyPlanContext<unknown>['countries']['filter'];
  readonly skillIcons: LeaderSkillIcons;
  readonly portraits?: Readonly<Record<string, string>>;
}

export const GENERALS_BASE: TableBase = {
  density: 'normal',
  barsBehind: 'all',
  overflow: 'scroll',
  showRank: false,
  sort: DEFAULT_SORT,
};

function parseSkillIcons(raw: unknown): LeaderSkillIcons {
  const icons = spriteIcons(raw, SKILL_SPRITES);
  const get = (sprite: string) => icons.get(spriteId({ key: sprite, frame: 0 })) ?? undefined;
  return {
    level: get('GFX_land_battle_leader_skill_bg'),
    attack: get('GFX_trait_attack'),
    defense: get('GFX_trait_defense'),
    planning: get('GFX_trait_planning'),
    logistics: get('GFX_trait_logistics'),
  };
}

export function generalsColumns(icons: LeaderSkillIcons = {}, attrLabels: readonly string[] = ['Attack', 'Defense', 'Planning', 'Logistics']): TableColumnSpec[] {
  return [
    {
      key: 'country',
      label: 'Country',
      as: 'flag',
      offered: OFFERED.country,
      showNamesDefault: false,
      pinned: true,
      priority: 0,
      text: true,
      hideable: false,
    },
    {
      key: 'leader',
      label: 'Commander',
      as: 'icon',
      cellKind: 'item',
      offered: OFFERED.text,
      pinned: true,
      priority: 0,
      text: true,
      hideable: false,
      minWidth: 196,
      widest: ['Erich von Manstein'],
    },
    {
      key: 'skill',
      label: 'Level',
      as: 'plain',
      cellKind: 'skill',
      skillIcon: icons.level,
      barEligible: false,
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      format: 'integer',
      minWidth: 66,
    },
    {
      key: 'attack',
      label: attrLabels[0] ?? 'Attack',
      as: 'plain',
      cellKind: 'skill',
      skillIcon: icons.attack,
      barEligible: false,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer',
      minWidth: 68,
    },
    {
      key: 'defense',
      label: attrLabels[1] ?? 'Defense',
      as: 'plain',
      cellKind: 'skill',
      skillIcon: icons.defense,
      barEligible: false,
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer',
      minWidth: 74,
    },
    {
      key: 'planning',
      label: attrLabels[2] ?? 'Planning',
      as: 'plain',
      cellKind: 'skill',
      skillIcon: icons.planning,
      barEligible: false,
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      format: 'integer',
      minWidth: 78,
    },
    {
      key: 'logistics',
      label: attrLabels[3] ?? 'Logistics',
      as: 'plain',
      cellKind: 'skill',
      skillIcon: icons.logistics,
      barEligible: false,
      offered: OFFERED.number,
      priority: 3,
      numeric: true,
      format: 'integer',
      minWidth: 78,
    },
    {
      key: 'traits',
      label: 'Traits',
      as: 'plain',
      cellKind: 'icons',
      offered: OFFERED.text,
      priority: 4,
      sortable: false,
      minWidth: 136,
    },
    {
      key: 'won',
      label: 'Won',
      as: 'faint',
      barEligible: true,
      tone: 'country',
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      format: 'integer',
      widest: ['999'],
    },
    {
      key: 'lost',
      label: 'Lost',
      as: 'faint',
      barEligible: true,
      tone: 'country',
      offered: OFFERED.number,
      priority: 2,
      numeric: true,
      format: 'integer',
      widest: ['999'],
    },
    {
      key: 'winRate',
      label: 'Win rate',
      as: 'plain',
      cellKind: 'winRate',
      barEligible: false,
      offered: OFFERED.number,
      priority: 1,
      numeric: true,
      minWidth: 116,
    },
    {
      key: 'fought',
      label: 'Fought',
      as: 'faint',
      barEligible: true,
      tone: 'country',
      offered: OFFERED.number,
      priority: 5,
      numeric: true,
      format: 'integer',
      defaultHidden: true,
      widest: ['999'],
    },
    {
      key: 'unresolved',
      label: 'Unresolved',
      as: 'faint',
      barEligible: true,
      offered: OFFERED.number,
      priority: 6,
      numeric: true,
      format: 'integer',
      defaultHidden: true,
      widest: ['999'],
    },
  ];
}

function passingCountriesAt(filter: GeneralsProjectionOptions['filter'], hour: number): ReadonlySet<string> | null {
  if (!filter.passing) return null;
  const set = new Set<string>();
  for (const [tag, ranges] of Object.entries(filter.passing)) {
    if (ranges.some(([first, last]) => first <= hour && hour <= last)) set.add(tag);
  }
  return set;
}

export const generalsProjection: TableProjection<GeneralsProjectionOptions> = registerTableProjection({
  kind: 'generals',
  family: 'table',
  project(readView: BoundedReadView, options: GeneralsProjectionOptions): TableModel {
    const hour = readView.hour;
    const baseCols = generalsColumns(options.skillIcons);
    const refused = readView.refused('main');
    if (refused !== undefined) {
      return { columns: baseCols, rows: [], hour, refused, rank: true, rowHeightMode: 'media' };
    }
    const rawPanel = readView.panel('main');
    if (rawPanel === undefined) {
      return { columns: baseCols, rows: [], hour, rank: true, rowHeightMode: 'media' };
    }
    const reply: UnitLeadersReply = validateLeaders(rawPanel);
    if (reply.missing) {
      return {
        columns: baseCols,
        rows: [],
        hour,
        empty: 'Leaders weren’t recorded at this time.',
        emptyDetail: reply.missing,
        emptyKind: 'unavailable',
        warnings: reply.warnings,
        rank: true,
        rowHeightMode: 'media',
      };
    }

    const unrecorded = options.filter.unrecorded.find(([first, last]) => first <= hour && hour <= last);
    if (unrecorded) {
      return {
        columns: baseCols,
        rows: [],
        hour,
        empty: unrecorded[2],
        emptyKind: 'unavailable',
        warnings: reply.warnings,
        rank: true,
        rowHeightMode: 'media',
      };
    }

    const allowedCountries = options.filterAll ? null : passingCountriesAt(options.filter, hour);
    const rows = leaderRows(reply, hour, allowedCountries, options.kind);
    const attrLabels = rows.length
      ? [0, 1, 2, 3].map(i => [...new Set(rows.map(r => attributeNames(r.role)[i]))].join(' / ') || ['Attack', 'Defense', 'Planning', 'Logistics'][i]!)
      : ['Attack', 'Defense', 'Planning', 'Logistics'];

    for (const url of Object.values(options.skillIcons)) {
      if (url) resolveArtUrlFromReadView(readView, url);
    }
    const columns = generalsColumns(options.skillIcons, attrLabels);

    const withheld = withheldLeaders(reply, allowedCountries, options.kind);
    if (!rows.length) {
      if (withheld.length) {
        const ws = withheldLeaderState(withheld);
        return {
          columns,
          rows: [],
          hour,
          empty: ws.message,
          emptyDetail: ws.detail,
          emptyKind: 'unavailable',
          warnings: reply.warnings,
          rank: true,
          rowHeightMode: 'media',
        };
      }
      return {
        columns,
        rows: [],
        hour,
        empty: 'No leaders for these countries at this time.',
        emptyKind: 'empty',
        warnings: reply.warnings,
        rank: true,
        rowHeightMode: 'media',
      };
    }

    const countVal = (n: number | null, partial: boolean) => {
      if (n === null) return notRecorded('Result not recorded');
      if (partial) return partialValue(n, 'Battle counts are incomplete: some battles were not recorded.');
      return n === 0 ? zeroValue(0) : knownValue(n);
    };

    const tableRows: TableRow[] = rows.map(row => {
      const ownerTag = row.role.owner;
      const country = resolveCountryFromReadView(readView, ownerTag, hour);
      const portraitUrl = options.portraits?.[row.key] ?? null;
      const portraitArt = portraitUrl ? resolveArtUrlFromReadView(readView, portraitUrl) : {};
      const traitItems: TableCellIconItem[] = row.traits.map(trait => {
        const name = contentTitle(trait.name, trait.token, 'Trait name unavailable');
        const resolved = resolveArtUrlFromReadView(readView, trait.icon);
        return { name, url: resolved.icon, pending: resolved.iconPending };
      });
      const rankLabel = row.role.rank ?? undefined;
      const comp = row.row.components;
      const cells: Record<string, TableCell> = {
        country: {
          value: country.value,
          iconSlot: true,
          icon: country.icon,
          iconPending: country.iconPending,
          country: ownerTag ?? undefined,
          countryColor: country.color,
        },
        leader: {
          value: knownValue(row.name),
          ...(rankLabel ? { subtext: rankLabel } : {}),
          iconSlot: true,
          artShape: 'portrait',
          artName: row.name,
          icon: portraitArt.icon,
          iconPending: portraitArt.iconPending ?? (portraitUrl === null && options.portraits === undefined),
        },
        skill: {
          value: row.row.skill === null ? notRecorded('Skill not recorded') : knownValue(row.row.skill),
        },
        attack: { value: comp[0] === undefined ? notRecorded('Not recorded') : knownValue(comp[0]) },
        defense: { value: comp[1] === undefined ? notRecorded('Not recorded') : knownValue(comp[1]) },
        planning: { value: comp[2] === undefined ? notRecorded('Not recorded') : knownValue(comp[2]) },
        logistics: { value: comp[3] === undefined ? notRecorded('Not recorded') : knownValue(comp[3]) },
        traits: {
          value: row.row.traits === null
            ? notRecorded('Traits not recorded')
            : knownValue(traitItems.map(t => t.name).join(', ') || 'None'),
          icons: traitItems,
          iconStripKind: 'traits',
        },
        won: { value: countVal(row.won, row.foughtPartial), country: ownerTag ?? undefined, countryColor: country.color },
        lost: { value: countVal(row.lost, row.foughtPartial), country: ownerTag ?? undefined, countryColor: country.color },
        winRate: {
          value: row.won === null ? notRecorded('Result not recorded') : knownValue(row.won),
          secondaryValue: row.lost === null ? notRecorded('Result not recorded') : knownValue(row.lost),
          sortValue: row.winRate === null ? notRecorded('No decided battles') : knownValue(row.winRate),
        },
        fought: { value: countVal(row.fought, row.foughtPartial), country: ownerTag ?? undefined, countryColor: country.color },
        unresolved: { value: countVal(row.unresolved, row.foughtPartial) },
      };

      const names = attributeNames(row.role);
      const extra: TooltipExtraRow[] = [
        { label: 'Country', value: country.value, divider: true },
        ...(rankLabel ? [{ label: 'Rank', value: knownValue(rankLabel) }] : []),
        { label: `${names[0]} / ${names[1]} / ${names[2]} / ${names[3]}`, value: knownValue(`${comp[0] ?? '—'} · ${comp[1] ?? '—'} · ${comp[2] ?? '—'} · ${comp[3] ?? '—'}`) },
        ...row.traits.map((trait, idx): TooltipExtraRow => ({
          label: contentTitle(trait.name, trait.token, 'Trait name unavailable'),
          value: knownValue(traitDateText(row.row.traitDates[idx] ?? null)),
          ...(idx === 0 ? { divider: true } : {}),
        })),
      ];

      return {
        key: row.key,
        name: row.name,
        cells,
        tooltipExtra: extra,
        tooltipAction: 'Click for leader career',
        meta: { reference: row.key, firstSeenHour: row.row.firstSeenHour, hour },
      };
    });

    return {
      columns,
      rows: tableRows,
      hour,
      rank: true,
      rowHeightMode: 'media',
      warnings: reply.warnings,
      notes: withheld.length ? [`${withheld.length} leaders withheld.`] : undefined,
    };
  },
});

function generalsSources(ctx: FamilyPlanContext<LeaderSkillIcons>): SourcePlan {
  const kind = (ctx.options.leaders === 'all' || ctx.options.leaders === 'navy' ? ctx.options.leaders : 'land') as LeaderKinds;
  const selection = ctx.countries.all ? leaderRequestSelection(null) : leaderRequestSelection(null);
  const owner = `generals:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}`;
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
        product: 'unit-leaders',
        followQuery: { panel: 'unit-leaders', branchId: ctx.branch.id, selection, leaders: kind },
        params: { branchId: ctx.branch.id, selection, leaders: kind },
        validate: validateLeaders,
      },
    }],
  };
}

export const generalsDeclaration: TableWidgetDeclaration<GeneralsProjectionOptions, LeaderSkillIcons> = {
  family: 'table',
  kind: 'generals',
  label: 'Generals leaderboard',
  subject: 'leaders',
  emptyCampaign: 'Load a campaign to see its leaders.',
  icon: 'star',
  optionFields: OPTIONS,
  countries: { mode: 'filter', labels: true, colours: true },
  aux: {
    product: 'sprite-artwork',
    params: () => ({ sprites: SKILL_SPRITES }),
    validate: parseSkillIcons,
  },
  sources: generalsSources,
  projection: generalsProjection,
  columns: ctx => generalsColumns(ctx.aux ?? {}),
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: GENERALS_BASE },
  projectionOptions: ctx => ({
    kind: (ctx.options.leaders === 'all' || ctx.options.leaders === 'navy' ? ctx.options.leaders : 'land') as LeaderKinds,
    filterAll: ctx.countries.all,
    filter: ctx.countries.filter,
    skillIcons: ctx.aux ?? {},
  }),
  jumpToDate: true,
  keyByBranch: true,
};
