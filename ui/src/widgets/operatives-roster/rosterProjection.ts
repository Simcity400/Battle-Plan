import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artNameCell, artState, identityFlag, identityName, newIdentityBoundary, urlArtHandle, type IdentityBoundary } from '../../families/table/identityCell.ts';
import type { TableCell, TableCellIconItem, TableColumnSpec, TableModel, TableProjection, TableRow, TooltipExtraRow } from '../../families/table/model.ts';
import type { StatusMarkKind } from '../../draw/kit.ts';
import { registerTableProjection } from '../../families/table/registry.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notRecorded, zeroValue, type Value } from '../../families/value.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { Country, OperativesRosterReply, OperativeTrait, RosterOperative } from '../../generated/dashboard';
import { COLUMNS, TABLE_BASE } from './definition.ts';

export type RosterProjectionOptions = Record<string, never>;

const object = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number => Number.isSafeInteger(value);
const finite = (value: unknown) => value === null || (typeof value === 'number' && Number.isFinite(value));
const text = (value: unknown) => value === null || typeof value === 'string';
const strings = (value: unknown): value is string[] => Array.isArray(value) && value.every(item => typeof item === 'string');
const country = (value: unknown): value is Country => object(value) && integer(value.id) && (value.tag === null || (typeof value.tag === 'string' && value.tag.length > 0));

function operativeOk(item: unknown, artwork: readonly unknown[]): item is RosterOperative {
  const art = (value: unknown) => value === null || (integer(value) && value >= 0 && value < artwork.length);
  return object(item) && typeof item.id === 'string' && /^\d{1,20}$/.test(item.id) && typeof item.roster === 'string' && country(item.owner)
    && text(item.name) && typeof item.unnamed === 'boolean' && (!item.unnamed || item.name === null) && text(item.nameMissing)
    && art(item.portrait) && text(item.portraitMissing) && (item.skill === null || integer(item.skill))
    && typeof item.experienceRaw === 'string' && /^-?\d{1,20}$/.test(item.experienceRaw) && text(item.experienceText) && finite(item.experienceRatio)
    && text(item.experienceMissing) && integer(item.assignment) && text(item.assignmentName) && text(item.assignmentDuration) && finite(item.assignmentProgress)
    && text(item.assignmentMissing) && art(item.assignmentIcon) && text(item.assignmentIconMissing) && (item.target === null || country(item.target))
    && typeof item.captured === 'boolean' && (item.capturer === null || country(item.capturer)) && (item.capturedHour === null || integer(item.capturedHour))
    && text(item.traitsMissing) && Array.isArray(item.traits)
    && item.traits.every(entry => object(entry) && text(entry.token) && text(entry.name) && art(entry.icon) && text(entry.missing));
}

export function validateRoster(value: unknown): OperativesRosterReply {
  if (!object(value) || !strings(value.artwork) || !Array.isArray(value.operatives) || !value.operatives.every(item => operativeOk(item, value.artwork as string[]))
    || !strings(value.unheld) || !(value.agencyFeature === null || typeof value.agencyFeature === 'boolean') || !(value.readHour === null || integer(value.readHour))
    || !text(value.missing) || !strings(value.warnings)) throw new Error('The operatives roster reply is malformed.');
  return value as unknown as OperativesRosterReply;
}

const PRIORITY: Readonly<Record<string, number>> = { operative: 0, skill: 1, state: 2, mission: 3, progress: 4, traits: 5 };
export const ROSTER_COLUMNS: readonly TableColumnSpec[] = COLUMNS.map(column => ({
  ...column,
  priority: PRIORITY[column.key] ?? 9,
  numeric: column.key === 'skill' || column.key === 'progress',
  text: column.key === 'operative' || column.key === 'mission' || column.key === 'state' || column.key === 'traits',
  ...(column.key === 'operative' ? { cellKind: 'item' as const, pinned: true } : {}),
  ...(column.key === 'skill' ? { barEligible: false, format: 'integer' as const, widest: ['9'] } : {}),
  ...(column.key === 'progress' ? { cellKind: 'track' as const, trackTone: 'accent' as const, widest: ['100%'] } : {}),
  ...(column.key === 'traits' ? { cellKind: 'icons' as const, sortable: false } : {}),
  ...(column.key === 'state' ? { cellKind: 'status' as const } : {}),
}));

const STATES: Readonly<Record<number, { readonly word: string; readonly mark: StatusMarkKind }>> = {
  0: { word: 'Captured', mark: 'ended' },
  1: { word: 'Cooling down', mark: 'waiting' },
  3: { word: 'On mission', mark: 'running' },
  4: { word: 'In operation', mark: 'running' },
};

export function operativeName(operative: Pick<RosterOperative, 'name' | 'unnamed' | 'nameMissing'>): Value<string> {
  if (operative.unnamed) return knownValue('Unnamed operative');
  return operative.name ? knownValue(operative.name) : notRecorded(operative.nameMissing ?? 'Name not recorded');
}

function stateCell(operative: RosterOperative): TableCell {
  const state = STATES[operative.assignment];
  if (state) return { value: knownValue(state.word), statusKind: state.mark };
  if (operative.assignmentName) return { value: knownValue(operative.assignmentName), statusKind: 'finished' };
  return { value: notRecorded(operative.assignmentMissing ?? 'State not recorded') };
}

function traitsCell(readView: BoundedReadView, operative: RosterOperative, artwork: readonly string[]): TableCell {
  if (operative.traitsMissing !== null) return { value: notRecorded(operative.traitsMissing) };
  const icons: TableCellIconItem[] = operative.traits.map((entry: OperativeTrait) => {
    const art = artState(readView, urlArtHandle(entry.icon === null ? null : artwork[entry.icon]));
    return { name: entry.name ?? entry.token ?? 'Trait not named', ...(art.icon ? { url: art.icon } : {}), ...(art.iconPending ? { pending: true } : {}) };
  });
  return { value: knownValue(operative.traits.length), icons, iconStripKind: 'traits' };
}

function progressCell(operative: RosterOperative): TableCell {
  const ratio = operative.experienceRatio;
  if (ratio === null) return { value: notRecorded(operative.experienceMissing ?? 'Progress not recorded'), bar: null };
  const share = Math.max(0, Math.min(1, ratio));
  const percent = Math.floor(share * 100);
  return { value: knownValue(`${percent}%`), sortValue: knownValue(share), bar: { fraction: share === 0 ? zeroValue(0) : knownValue(share) } };
}

function missionCell(readView: BoundedReadView, operative: RosterOperative, boundary: IdentityBoundary): TableCell {
  const name = operative.assignmentName;
  const value = name ? knownValue(name) : notRecorded(operative.assignmentMissing ?? 'Mission not recorded');
  const tag = operative.target?.tag ?? null;
  return tag === null ? { value } : { value, flags: [identityFlag(readView, tag, { boundary })] };
}

function targetName(readView: BoundedReadView, tag: string | null | undefined): Value<string> {
  return tag ? knownValue(identityName(readView, 'country', tag)) : notRecorded('Not recorded');
}

function rosterExtra(readView: BoundedReadView, operative: RosterOperative): readonly TooltipExtraRow[] {
  return [
    ...(operative.experienceText ? [{ label: 'Experience', value: knownValue(operative.experienceText), divider: true }] : []),
    ...(operative.target ? [{ label: 'Target', value: targetName(readView, operative.target.tag) }] : []),
    ...(operative.assignmentDuration ? [{ label: 'Days left', value: knownValue(operative.assignmentDuration) }] : []),
    ...(operative.captured ? [
      { label: 'Captured by', value: targetName(readView, operative.capturer?.tag) },
      { label: 'Captured on', value: operative.capturedHour === null ? notRecorded('Capture date not recorded') : knownValue(formatDateOnly(operative.capturedHour)) },
    ] : []),
  ];
}

export const rosterProjection: TableProjection<RosterProjectionOptions> = registerTableProjection({
  kind: 'operatives-roster',
  family: 'table',
  project(readView, _options): TableModel {
    const hour = readView.hour;
    const refused = readView.refused('main');
    if (refused !== undefined) return { columns: ROSTER_COLUMNS, rows: [], hour, refused };
    const raw = readView.panel('main');
    if (raw === undefined) return { columns: ROSTER_COLUMNS, rows: [], hour };
    const reply = validateRoster(raw);
    if (reply.missing !== null) {
      return { columns: ROSTER_COLUMNS, rows: [], hour, empty: 'Not recorded', emptyDetail: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
    }
    if (reply.agencyFeature === false) {
      return { columns: ROSTER_COLUMNS, rows: [], hour, empty: 'No intelligence agencies', emptyDetail: 'This recording was made without intelligence agencies', emptyKind: 'empty', warnings: reply.warnings };
    }
    const boundary = newIdentityBoundary();
    const rows: TableRow[] = reply.operatives.map(operative => {
      const name = operativeName(operative);
      const portrait = operative.portrait === null ? null : reply.artwork[operative.portrait] ?? null;
      return {
        key: JSON.stringify([operative.roster, operative.id]),
        name: name.kind === 'known' ? name.value : operative.id,
        cells: {
          operative: artNameCell(readView, name.kind === 'known' ? name.value : null, urlArtHandle(portrait), operative.nameMissing ?? 'Name not recorded', 'portrait'),
          skill: { value: operative.skill === null ? notRecorded('Skill not recorded') : knownValue(operative.skill) },
          progress: progressCell(operative),
          mission: missionCell(readView, operative, boundary),
          traits: traitsCell(readView, operative, reply.artwork),
          state: stateCell(operative),
        },
        tooltipExtra: () => rosterExtra(readView, operative),
      };
    });
    return {
      columns: ROSTER_COLUMNS,
      rows,
      hour,
      rowHeightMode: 'media',
      nextChangeAt: boundary.next,
      rowTimeBoundaries: false,
      ...(rows.length === 0
        ? reply.unheld.length
          ? { empty: 'Not recorded', emptyDetail: 'No country holds this tag at this date', emptyKind: 'unavailable' as const }
          : { empty: 'No operatives', emptyKind: 'empty' as const }
        : {}),
      warnings: reply.warnings,
    };
  },
});

export const rosterDeclaration: TableWidgetDeclaration<RosterProjectionOptions> = {
  family: 'table',
  kind: 'operatives-roster',
  label: 'Operatives roster',
  subject: 'operatives',
  emptyCampaign: 'Load a campaign to see operatives.',
  icon: 'shield',
  countries: { mode: 'single', choose: 'Choose a country to see its operatives.' },
  sources: ctx => {
    const params = ctx.countryTag ? { branchId: ctx.branch.id, countries: [countryFromKey(ctx.countryTag)] } : null;
    return {
      owner: `operatives-roster:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${ctx.countryTag ?? ''}`,
      entries: params
        ? [{
            name: 'main',
            source: ctx.track.source,
            branch: ctx.branch.id,
            offset: ctx.track.axisOffset,
            read: {
              shape: 'point',
              channel: 'panel',
              coverage: { mode: 'keyed', keys: [ctx.countryTag!] },
              product: 'operatives-roster',
              followQuery: { panel: 'operatives-roster', ...params },
              params,
              validate: validateRoster,
            },
          }]
        : [],
    };
  },
  projection: rosterProjection,
  columns: ROSTER_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: TABLE_BASE },
  projectionOptions: () => ({}),
};
