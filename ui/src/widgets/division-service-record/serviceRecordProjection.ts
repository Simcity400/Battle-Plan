import { subjectHandle } from '../../dashboard/chosen.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TableWidgetDeclaration } from '../../families/table/declaration.ts';
import { artKindCell, LOADING_NAME, newIdentityBoundary, resolveIdentity, resolveName, type IdentityBoundary } from '../../families/table/identityCell.ts';
import { LOG_TABLE_BASE, logCountryCell, logDateCell, logStatusCell, occurrenceSelection, pagedTailEntry, projectPagedSlots } from '../../families/table/logHelpers.ts';
import type { TableCell, TableModel, TableProjection, TableRow, TableSubjectPicture } from '../../families/table/model.ts';
import { TABLE_SETTINGS_KEY } from '../../families/table/settings.ts';
import { knownValue, notApplicable, notRecorded } from '../../families/value.ts';
import type { CareerFacts, ServiceEntry, ServiceRecordTailReply } from '../../generated/dashboard';
import { nameOf, removals, serviceRecordPicker } from './serviceRecordChoices.ts';
import { SERVICE_COLUMNS } from './definition.ts';
import { ENEMY_KILLED_NOTE } from '../most-decorated-divisions/definition.ts';

interface Options {
  readonly campaign: string;
  readonly chosen: string;
}

const base = { columns: SERVICE_COLUMNS, hideHeader: true, unsorted: true, dateGrouped: true, rowTimeBoundaries: false } as const;
const blank: TableCell = { value: notApplicable('Does not apply to this entry'), blank: true };
const BATTLE_ART = { kind: 'sprite', key: 'GFX_landbattle_strip', frame: 0 } as const;

function kindCell(view: BoundedReadView, row: ServiceEntry, sprite: string | null, name: string): TableCell {
  if (row.kind === 'award') {
    const icon = row.medal?.icon;
    return artKindCell(view, icon ? { kind: 'sprite', key: icon.key, frame: icon.frame } : null, 'drawn', row.medal?.name ?? name,
      row.medal?.iconMissing ?? 'Medal artwork not recorded');
  }
  if (row.kind === 'battle' || row.kind === 'career') return artKindCell(view, BATTLE_ART, 'square', name, 'Land battle artwork not in your installation');
  return artKindCell(view, sprite ? { kind: 'sprite', key: sprite, frame: 0 } : null, 'drawn', name, 'Division artwork not recorded');
}

function serviceRow(view: BoundedReadView, row: ServiceEntry, sprite: string | null, boundary: IdentityBoundary): TableRow {
  const award = row.kind === 'award';
  const created = row.kind === 'created';
  const removed = row.kind === 'removed';
  const countries = removed && row.other ? [{ tag: row.other }, { tag: row.tag, dimmed: true }]
    : [{ tag: row.tag }, ...(row.other && row.other !== row.tag ? [{ tag: row.other }] : [])];
  const status = removed ? removals[row.status ?? ''] : row.status;
  const place = row.province !== null && row.placeHour !== null
    ? resolveName(view, 'province', String(row.province), { atHour: row.placeHour, boundary }).value : undefined;
  const title = row.kind === 'battle' && place?.kind === 'known' ? `Battle of ${place.value}` : created ? 'First seen' : removed ? 'Division removed' : row.title;
  const battlePending = row.kind === 'battle' && title === null && place?.kind === 'pending';
  const name = title ?? (battlePending ? LOADING_NAME : award ? 'Medal name not recorded' : 'Entry description not recorded');
  return {
    key: row.key, name, rowHour: row.hour, meta: row.hour,
    cells: {
      date: logDateCell(row.hour),
      kind: kindCell(view, row, sprite, name),
      country: logCountryCell(view, countries, row.observed, boundary),
      item: { value: title ? knownValue(title) : battlePending ? place : notRecorded(name), missingFigure: award ? 'Medal' : 'Entry' },
      place: place ? { value: place }
        : award ? blank : { value: notRecorded('Place not recorded'), missingFigure: 'Place' },
      status: status ? logStatusCell(status, removed ? 'ended' : 'finished') : removed ? { value: notRecorded('Removal cause not recorded') } : blank,
    },
  };
}

function honoursOf(view: BoundedReadView, head: ServiceRecordTailReply, facts: CareerFacts): TableSubjectPicture[] {
  return facts.awards.map(award => {
    const medal = award.medal == null ? null : head.medals[award.medal] ?? null;
    const name = medal?.name ?? 'Medal not identified';
    const art = medal?.icon ? view.art({ kind: 'sprite', key: medal.icon.key, frame: medal.icon.frame }) : null;
    const count = award.count > 1 ? { count: award.count } : {};
    if (art?.status === 'ready') return { name, url: art.url, ...count };
    return { name, missing: medal?.iconMissing ?? (art?.status === 'pending' ? 'Medal artwork loading' : 'Medal artwork not in your installation'), ...count };
  });
}

export const serviceRecordProjection: TableProjection<Options> = registerFamilyProjection({
  kind: 'division-service-record', family: 'table',
  project(view, options): TableModel {
    const hour = view.hour;
    const source = view.rows<ServiceEntry>('main');
    const refused = view.refused('main') ?? source?.refused;
    if (!source || refused) return { ...base, hour, rows: [], ...(refused ? { refused } : {}) };
    const head = source.rawHead as ServiceRecordTailReply | undefined;
    const boundary = newIdentityBoundary();
    const chosen = head?.selected;
    const spriteUrl = (key: string | null | undefined, frame = 0) => {
      const art = key ? view.art({ kind: 'sprite', key, frame }) : null;
      return art?.status === 'ready' ? art.url : undefined;
    };
    const icon = spriteUrl(chosen?.sprite);
    const facts = head?.facts;
    const insignia = facts?.level == null ? undefined : spriteUrl('GFX_unit_level', facts.level);
    const country = chosen?.tag ? resolveIdentity(view, 'country', chosen.tag, { atHour: chosen.lastSeen ?? hour, boundary }) : null;
    const filter = { key: 'division', noun: 'divisions', allLabel: 'Follow most decorated', items: [], selected: options.chosen ? [options.chosen] : [],
      single: { selected: chosen?.division, pinned: !!options.chosen, countryTag: chosen?.tag ?? undefined, countryLabel: country?.name, countryFlag: country?.art?.status === 'ready' ? country.art.url : undefined,
        label: chosen ? nameOf(chosen) : 'Choose division', ...(icon ? { icon } : {}) } };
    if (options.chosen && subjectHandle(options.campaign, options.chosen) === null) {
      return { ...base, hour, rows: [], kindFilter: filter, emptyKind: 'unavailable', empty: 'The chosen division belongs to another campaign.' };
    }
    const page = projectPagedSlots(source, hour, view.rowWindow, row => serviceRow(view, row, chosen?.sprite ?? null, boundary));
    const unavailable = head?.unavailable ?? facts?.historyMissing;
    return {
      ...base, ...page, hour, nextChangeAt: boundary.next, kindFilter: filter, warnings: head?.warnings,
      ...(chosen && head ? { subjectSummary: {
        name: chosen.name ? knownValue(chosen.name) : notRecorded('Division name not recorded'), ...(icon ? { icon } : {}),
        country: country ? country.value : notRecorded('Country not recorded'), ...(country?.art?.status === 'ready' ? { flag: country.art.url } : {}),
        detail: chosen.template ? knownValue(chosen.template) : notRecorded('Template not recorded'),
        level: facts?.levelName ? knownValue(facts.levelName) : notRecorded('Experience not recorded'), ...(insignia ? { insignia } : {}),
        figures: [
          ...(facts?.medals === 0 ? [] : [{ label: 'Honours', value: facts?.medals == null ? notRecorded(facts?.historyMissing ?? 'Honours not recorded') : knownValue(facts.medals),
            ...(facts?.awards.length ? { pictures: honoursOf(view, head, facts) } : {}) }]),
          { label: 'Casualties taken', value: facts?.casualtiesTaken == null ? notRecorded('Casualties taken not recorded') : knownValue(facts.casualtiesTaken) },
          { label: 'Enemy killed', value: facts?.enemyKilled == null ? notRecorded(facts?.enemyKilledMissing ?? 'Enemy killed not recorded') : knownValue(facts.enemyKilled), hint: ENEMY_KILLED_NOTE },
        ],
      } } : {}),
      ...(page.slots.length ? {} : unavailable
        ? { empty: unavailable, emptyKind: 'unavailable' as const }
        : { empty: chosen ? 'No entries recorded for this division.' : 'No decorated divisions recorded. Choose a division to see its record.' }),
    };
  },
});

export const serviceRecordDeclaration: TableWidgetDeclaration<Options> = {
  picker: serviceRecordPicker,
  family: 'table', kind: 'division-service-record', jumpToDate: true, label: 'Division service record', subject: 'division', icon: 'medal',
  emptyCampaign: 'Load a campaign to see a division’s record.', countries: { mode: 'filter' }, columns: SERVICE_COLUMNS,
  settings: { kind: 'table', key: TABLE_SETTINGS_KEY, base: LOG_TABLE_BASE },
  sources(ctx) {
    const division = subjectHandle(ctx.track.source.metadata.campaignKey, String(ctx.options.division ?? ''));
    const ask = { ...occurrenceSelection(ctx), ...(division ? { division } : {}),
      show: ctx.options.show === 'career' || ctx.options.show === 'awards' ? ctx.options.show : 'both' };
    return { owner: JSON.stringify([ctx.track.source.id, ctx.branch.id, 'division-service-record', ask]),
      entries: [pagedTailEntry('main', ctx.track.source, ctx.branch.id, ctx.track.axisOffset,
        { product: 'division-service-record-tail', branchId: ctx.branch.id, from: null, limit: 64, ask }, ctx.following)] };
  },
  projection: serviceRecordProjection,
  projectionOptions: ctx => ({ campaign: ctx.track.source.metadata.campaignKey, chosen: String(ctx.options.division ?? '') }),
};
