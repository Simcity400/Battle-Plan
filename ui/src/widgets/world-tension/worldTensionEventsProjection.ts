import type { FamilyPlanContext } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableColumnSpec,
  type TableProjection,
} from '../../families/table/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, type Value } from '../../families/value.ts';
import { formatDateOnly, formatExactRational, formatScaledPercent } from '../../families/valueFormat.ts';
import type { TensionStackEvent, WorldTensionStackReply } from '../../generated/dashboard';
import { EVENTS_TABLE_BASE } from './definition.ts';
import { validateWorldTensionStack } from './worldTensionProjection.ts';

export interface WorldTensionEventsOptions {
  readonly branchFirst: number;
}

interface ParsedEvents {
  readonly events: readonly TensionStackEvent[];
  readonly scale: number;
  readonly refused: string | null;
  readonly listingMissing?: string;
  readonly nextChangeHour: number | null;
  readonly warnings: readonly string[];
}

function parseEvents(raw: unknown): ParsedEvents {
  const reply = validateWorldTensionStack(raw);
  return {
    events: reply.events,
    scale: reply.contributionScale,
    refused: reply.refused,
    listingMissing: reply.listingMissing,
    nextChangeHour: reply.nextChangeHour,
    warnings: reply.warnings,
  };
}

function formatAddedPercent(raw: string, scale: number, inspect = false): Value<string> {
  if (!/^-?\d+$/.test(raw) || !Number.isSafeInteger(scale) || scale <= 0) return notRecorded('Tension amount not recorded');
  return knownValue(inspect
    ? formatExactRational(BigInt(raw), BigInt(scale), { signed: true, percent: true })
    : formatScaledPercent(BigInt(raw), BigInt(scale), { signed: true }));
}

export function worldTensionEventsColumns(): readonly TableColumnSpec[] {
  return [
    { key: 'date', label: 'Created', as: 'plain', offered: OFFERED.text, priority: 0, widest: ['31 Dec 1945'] },
    { key: 'country', label: 'Country', as: 'flagName', offered: OFFERED.country, showNamesDefault: true, priority: 0, text: true, widest: ['United Kingdom'] },
    { key: 'reason', label: 'Event', as: 'plain', offered: OFFERED.text, priority: 0, text: true, hideable: false, widest: ['Declared war on Poland'] },
    { key: 'added', label: 'Adds', as: 'faint', barEligible: true, offered: OFFERED.number, priority: 1, numeric: true, widest: ['+100.0%'] },
    { key: 'until', label: 'Listed until', as: 'plain', offered: OFFERED.text, priority: 2, widest: ['Still listed'] },
  ];
}

export const worldTensionEventsProjection: TableProjection<WorldTensionEventsOptions> = registerFamilyProjection(
  declareTableProjection<ParsedEvents, TensionStackEvent, WorldTensionEventsOptions>({
    kind: 'world-tension',
    validate: parseEvents,
    nextChange: readView => {
      const reply = readView.panel<WorldTensionStackReply>('main');
      return reply?.nextChangeHour ?? Infinity;
    },
    columns: (parsed): readonly DeclarativeColumnSpec<TensionStackEvent, WorldTensionEventsOptions>[] => {
      const specs = worldTensionEventsColumns();
      return specs.map((spec): DeclarativeColumnSpec<TensionStackEvent, WorldTensionEventsOptions> => {
        if (spec.key === 'date') {
          return { ...spec, value: row => knownValue(formatDateOnly(row.hour)), sortValue: row => knownValue(row.hour) };
        }
        if (spec.key === 'country') {
          return {
            ...spec,
            countryField: 'tag',
            lookHour: (row, hour) => Math.min(row.firstRead, hour),
            whenNull: { kind: 'notApplicable', reason: 'Unknown country' },
          };
        }
        if (spec.key === 'reason') {
          return { ...spec, value: row => knownValue(row.reason) };
        }
        if (spec.key === 'added') {
          return {
            ...spec,
            country: 'tag',
            lookHour: (row, hour) => Math.min(row.firstRead, hour),
            value: row => formatAddedPercent(row.addedRaw, parsed.scale),
            extra: row => ({ tooltipRows: [{ label: spec.label, value: formatAddedPercent(row.addedRaw, parsed.scale, true) }] }),
            sortValue: row => {
              const num = Number(row.addedRaw);
              return Number.isFinite(num) && parsed.scale > 0 ? knownValue(num / parsed.scale) : notRecorded('Tension amount not recorded');
            },
          };
        }
        return {
          ...spec,
          value: row => row.lastRead !== null ? knownValue(formatDateOnly(row.lastRead))
            : parsed.listingMissing ? notRecorded(parsed.listingMissing) : knownValue('Still listed'),
          sortValue: (row, _live, hour) => row.lastRead === null && parsed.listingMissing ? notRecorded(parsed.listingMissing) : knownValue(row.lastRead ?? hour + 1),
        };
      });
    },
    rows: (parsed, _live, hour) => {
      if (parsed.refused) {
        return { rows: [], empty: parsed.refused, emptyKind: 'unavailable', warnings: parsed.warnings };
      }
      const shown = parsed.events.filter(ev => ev.firstRead <= hour && ev.hour <= hour);
      if (!shown.length) {
        if (parsed.listingMissing) return { rows: [], empty: parsed.listingMissing, emptyKind: 'unavailable', warnings: parsed.warnings };
        return { rows: [], empty: `No threat event recorded by ${formatDateOnly(hour)}.`, emptyKind: 'empty', warnings: parsed.warnings };
      }
      return { rows: shown, warnings: parsed.warnings };
    },
    rowKey: row => `${row.hour}:${row.firstRead}:${row.country}:${row.secondCountry}:${row.place}:${row.reason}`,
    rowName: row => row.reason,
    tooltipTitle: row => row.reason,
    tooltipNote: row => (row.remembered ? 'Country tag last held this slot before disappearing.' : undefined),
  }),
  'events',
);

export const worldTensionEventsDeclaration: TableWidgetDeclaration<WorldTensionEventsOptions> = {
  family: 'table',
  kind: 'world-tension',
  label: 'Threat events',
  subject: 'threat events',
  emptyCampaign: 'Load a campaign to see threat events.',
  icon: 'alert',
  countries: { mode: 'multi', content: 'history' },
  sources: (ctx: FamilyPlanContext, view) => {
    const top = ctx.countries.top ?? null;
    const keyed = !ctx.countries.all && top == null;
    const group = keyed ? groupSelection(ctx.widget.countries) : null;
    const countries = keyed && !group ? [...ctx.countries.tags] : null;
    const product = 'world-tension-stack';
    const coverage = { mode: 'population', form: 'all' } as const;
    const params = {
      branchId: ctx.branch.id,
      layers: 1,
      values: 'share',
      countries,
      top,
      group,
      width: 120,
      includeStack: false,
      includeEvents: true,
    };
    const followed = { ...params, firstHour: ctx.branch.firstHour };
    return {
      owner: `world-tension-events:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: view.following ? {
          shape: 'point',
          channel: 'panel',
          coverage,
          product,
          followQuery: { panel: product, ...followed },
          params: followed,
          hourParam: 'lastHour',
          validate: validateWorldTensionStack,
        } : {
          shape: 'branch-to-hour',
          coverage,
          product,
          followQuery: null,
          params,
          dynamicKeys: ['firstHour', 'lastHour'],
          validate: validateWorldTensionStack,
          provenInterval: (data, _requested, hour) => {
            const reply = data as WorldTensionStackReply;
            return reply.nextChangeHour !== null ? [hour, reply.nextChangeHour - 1] : [hour, Infinity];
          },
        },
      }],
    };
  },
  projection: worldTensionEventsProjection,
  columns: () => [...worldTensionEventsColumns()],
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: EVENTS_TABLE_BASE },
  projectionOptions: ctx => ({ branchFirst: ctx.branch.firstHour }),
  keyByBranch: true,
};
