import type { FamilyPlanContext } from '../../families/contract.ts';
import type { CountryLooksMap } from '../../families/looks.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import {
  declareTableProjection,
  type DeclarativeColumnSpec,
  type TableColumnSpec,
  type TableProjection,
} from '../../families/table/model.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { OFFERED, TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue } from '../../families/value.ts';
import { formatDateOnly, formatRecordedPercent } from '../../families/valueFormat.ts';
import type { TensionStackEvent, WorldTensionStackReply } from '../../generated/dashboard';
import { EVENTS_TABLE_BASE } from './definition.ts';
import { validateWorldTensionStack } from './worldTensionProjection.ts';

export interface WorldTensionEventsOptions {
  readonly branchFirst: number;
  readonly labels?: CountryLooksMap;
}

interface ParsedEvents {
  readonly events: readonly TensionStackEvent[];
  readonly scale: number;
  readonly nextChangeHour: number | null;
  readonly warnings: readonly string[];
}

function parseEvents(raw: unknown): ParsedEvents {
  const reply = validateWorldTensionStack(raw);
  return {
    events: reply.events,
    scale: reply.contributionScale,
    nextChangeHour: reply.nextChangeHour,
    warnings: reply.warnings,
  };
}

function formatAddedPercent(raw: string, scale: number): string {
  const num = Number(raw);
  if (!Number.isFinite(num) || scale <= 0) return '0%';
  return formatRecordedPercent(num / scale, { signed: true });
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

export const worldTensionEventsProjection: TableProjection<WorldTensionEventsOptions> = registerTableProjection(
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
            value: row => knownValue(formatAddedPercent(row.addedRaw, parsed.scale)),
            sortValue: row => knownValue(Number(row.addedRaw) / Math.max(1, parsed.scale)),
          };
        }
        return {
          ...spec,
          value: row => (row.lastRead === null ? knownValue('Still listed') : knownValue(formatDateOnly(row.lastRead))),
          sortValue: (row, _live, hour) => knownValue(row.lastRead === null ? hour + 1 : row.lastRead),
        };
      });
    },
    rows: (parsed, _live, hour) => {
      const shown = parsed.events.filter(ev => ev.firstRead <= hour && ev.hour <= hour);
      if (!shown.length) {
        return { rows: [], empty: `No threat event recorded by ${formatDateOnly(hour)}.`, emptyKind: 'empty', warnings: parsed.warnings };
      }
      return { rows: shown, warnings: parsed.warnings };
    },
    rowKey: row => `${row.hour}:${row.firstRead}:${row.reason}:${row.tag ?? ''}`,
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
  countries: { mode: 'multi', labels: true, colours: true, allTagsForLabels: true },
  sources: (ctx: FamilyPlanContext) => ({
    owner: `world-tension-events:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'branch-to-hour',
        coverage: { mode: 'population', form: 'all' },
        product: 'world-tension-stack',
        followQuery: null,
        params: {
          branchId: ctx.branch.id,
          layers: 1,
          values: 'share',
          countries: ctx.countries.all ? null : [...ctx.countries.tags],
          width: 120,
        },
        dynamicKeys: ['firstHour', 'lastHour'],
        validate: validateWorldTensionStack,
        provenInterval: (data, _requested, hour) => {
          const reply = data as WorldTensionStackReply;
          return reply.nextChangeHour !== null ? [hour, reply.nextChangeHour - 1] : [hour, Infinity];
        },
      },
    }],
  }),
  projection: worldTensionEventsProjection,
  columns: () => [...worldTensionEventsColumns()],
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: EVENTS_TABLE_BASE },
  projectionOptions: ctx => ({ branchFirst: ctx.branch.firstHour, labels: ctx.countries.labels }),
  keyByBranch: true,
};
