import { formatDateTime } from '../../dashboard/calendar.ts';
import type { MissingSpan } from '../../generated/dashboard';
import type { FamilySources } from '../../families/contract.ts';
import { logLookAt, type LogLooksMap, type LogProjection } from '../../families/log/declaration.ts';
import type { LogColumnTrack, LogFacetPill, LogModel, LogRowSpec, LogTextPart, LogTone, LogTooltipField } from '../../families/log/model.ts';
import { registerLogProjection } from '../../families/log/registry.ts';
import {
  cellAt,
  cellNumber,
  cellText,
  columnIndex,
  facetTotalMap,
  isLogTailReply,
  nonEmptyText,
  parseOrdinal,
} from '../../families/log/tailReply.ts';
import { knownValue, notApplicable, notRecorded, type Value } from '../../families/value.ts';
import { divisionKindFromNumber, divisionStatus, type DivisionKind } from './divisionLogData.ts';

export const DIVISION_LOG_PROJECTION_KIND = 'division-log';

export interface DivisionLogProjectionOptions {
  readonly events: string;
  readonly showTotals: boolean;
  readonly lossesAvailable: boolean;
  readonly looks: LogLooksMap;
  readonly manifestWarnings: readonly string[];
  readonly manifestGaps: readonly MissingSpan[];
}

const COLUMNS: readonly LogColumnTrack[] = [
  { id: 'date', width: 68 },
  { id: 'art', width: 24 },
  { id: 'body', flex: 1, minWidth: 180 },
  { id: 'status', width: 120, align: 'end', priority: 2 },
];

function kindTone(kind: DivisionKind | null): LogTone {
  switch (kind) {
    case 'created': return 'positive';
    case 'lost': return 'negative';
    case 'transfer': return 'accent';
    default: return 'muted';
  }
}

function countryValue(looks: LogLooksMap, tag: string | null, hour: number, missingReason: string): { readonly value: Value<string>; readonly look: ReturnType<typeof logLookAt>; readonly spoken: string } {
  if (!tag) return { value: notRecorded(missingReason), look: null, spoken: missingReason };
  const look = logLookAt(looks, tag, hour);
  const resolved = nonEmptyText(look?.name);
  if (resolved) return { value: knownValue(resolved), look, spoken: resolved };
  if (look?.status) return { value: notRecorded(look.status), look, spoken: look.status };
  return { value: knownValue(tag), look, spoken: tag };
}

function mergeGaps(a: readonly MissingSpan[], b: readonly MissingSpan[], upToHour: number): MissingSpan[] {
  const out: MissingSpan[] = [];
  const seen = new Set<string>();
  for (const list of [a, b]) {
    for (const span of list) {
      if (span.firstHour > upToHour) continue;
      const lastHour = Math.min(span.lastHour, upToHour);
      const key = `${span.firstHour}:${lastHour}:${span.reason}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ firstHour: span.firstHour, lastHour, reason: span.reason });
    }
  }
  out.sort((x, y) => x.firstHour - y.firstHour || x.lastHour - y.lastHour);
  return out;
}

export const divisionLogProjection: LogProjection<DivisionLogProjectionOptions> = {
  family: 'log',
  kind: DIVISION_LOG_PROJECTION_KIND,
  layout: 'row',
  project(sources: FamilySources, hour: number, options: DivisionLogProjectionOptions): LogModel {
    const rawTail = sources.panels.get('tail');
    const offsetRaw = sources.panels.get('__offset');
    const offset = typeof offsetRaw === 'number' && Number.isFinite(offsetRaw) ? offsetRaw : 0;

    if (!isLogTailReply(rawTail)) {
      return {
        hour,
        total: 0,
        baseOrdinal: 0,
        firstOrdinal: 0,
        offset: 0,
        layout: 'row',
        columns: COLUMNS,
        rows: [],
        gaps: [],
        empty: { kind: 'empty', message: 'No recorded division events match these settings.' },
        warnings: [...options.manifestWarnings],
      };
    }

    const index = columnIndex(rawTail.columns);
    const total = parseOrdinal(rawTail.total);
    const baseOrdinal = parseOrdinal(rawTail.baseOrdinal);
    const firstOrdinal = parseOrdinal(rawTail.firstOrdinal);
    const totals = facetTotalMap(rawTail);

    const rows: LogRowSpec[] = new Array(rawTail.rows.length);
    for (let i = 0; i < rawTail.rows.length; i++) {
      const wire = rawTail.rows[i]!;
      const readHour = cellNumber(cellAt(wire, index, 'readHour')) ?? wire.hour;
      const eventHour = cellNumber(cellAt(wire, index, 'hour')) ?? readHour;
      const by = cellNumber(cellAt(wire, index, 'by')) === 1;
      const afterHour = cellNumber(cellAt(wire, index, 'afterHour'));
      const kind = divisionKindFromNumber(cellNumber(cellAt(wire, index, 'kind')));
      const statusRaw = nonEmptyText(cellText(cellAt(wire, index, 'status')));
      const tag = nonEmptyText(cellText(cellAt(wire, index, 'tag')));
      const nameRaw = nonEmptyText(cellText(cellAt(wire, index, 'name')));
      const nameMissing = nonEmptyText(cellText(cellAt(wire, index, 'nameMissing')));
      const icon = nonEmptyText(cellText(cellAt(wire, index, 'icon')));
      const iconMissing = nonEmptyText(cellText(cellAt(wire, index, 'iconMissing')));
      const templateName = nonEmptyText(cellText(cellAt(wire, index, 'templateName')));
      const templateMissing = nonEmptyText(cellText(cellAt(wire, index, 'templateMissing')));
      const province = cellNumber(cellAt(wire, index, 'province'));
      const placeHour = cellNumber(cellAt(wire, index, 'placeHour'));
      const placeAtEvent = cellNumber(cellAt(wire, index, 'placeAtEvent')) === 1;
      const placeName = nonEmptyText(cellText(cellAt(wire, index, 'placeName')));
      const placeMissing = nonEmptyText(cellText(cellAt(wire, index, 'placeMissing')));
      const killerTag = nonEmptyText(cellText(cellAt(wire, index, 'killerTag')));

      const country = countryValue(options.looks, tag, readHour, 'Country not recorded');
      const killer = kind === 'created'
        ? { value: notApplicable('Division creation has no attacker') as Value<string>, look: null, spoken: 'Not applicable' }
        : countryValue(options.looks, killerTag, readHour, 'Killer not recorded');
      const nameVal: Value<string> = nameRaw
        ? knownValue(nameRaw)
        : notRecorded(nameMissing ?? 'Division name not recorded');
      const templateVal: Value<string> = templateName
        ? knownValue(templateName)
        : notRecorded(templateMissing ?? 'Template name not recorded');
      const placeVal: Value<string> = province === null || placeHour === null
        ? notRecorded('Location not recorded')
        : placeName
          ? knownValue(placeName)
          : notRecorded(placeMissing ?? 'Unnamed province');
      const kindVal: Value<string> = kind !== null
        ? knownValue(kind)
        : notRecorded('Division event kind not recorded');
      const statusText = statusRaw ? divisionStatus({ status: statusRaw }) : null;
      const statusVal: Value<string> = statusText
        ? knownValue(statusText)
        : notRecorded('Removal cause not recorded');

      const parts: LogTextPart[] = [
        { text: country.value, flag: tag ? country.look : undefined, weight: 'bold' },
        { text: knownValue(': '), tone: 'muted' },
        { text: nameVal, weight: 'bold' },
        { text: knownValue(' · '), tone: 'muted' },
        { text: templateVal, tone: 'muted' },
        { text: knownValue(placeAtEvent ? ' · at ' : ' · last seen at '), tone: 'muted' },
        { text: placeVal, tone: 'muted' },
      ];
      if (killerTag !== null) {
        parts.push(
          { text: knownValue(' · by '), tone: 'muted' },
          { text: killer.value, flag: killer.look, tone: 'muted' },
        );
      }

      const nameSpoken = nameRaw ?? nameMissing ?? 'Division name not recorded';
      const templateSpoken = templateName ?? templateMissing ?? 'Template not recorded';
      const placeSpoken = province === null || placeHour === null
        ? 'Location not recorded'
        : placeName ?? placeMissing ?? 'Unnamed province';
      const statusSpoken = statusText ?? 'Status not recorded';
      const spoken = `${country.spoken}: ${nameSpoken}, ${statusSpoken}, ${templateSpoken}, ${placeSpoken}${killerTag ? `, by ${killer.spoken}` : ''}.`;

      rows[i] = {
        key: wire.key,
        hour: readHour,
        by: false,
        ordinal: parseOrdinal(wire.ordinal),
        art: {
          ...(icon ? { icon } : {}),
          ...(iconMissing ? { missingReason: iconMissing } : {}),
        },
        slots: [
          { kind: 'rich', parts },
          { kind: 'pill', label: statusVal, tone: kindTone(kind) },
        ],
        auditCells: {
          kind: kindVal,
          status: statusVal,
          country: country.value,
          name: nameVal,
          template: templateVal,
          place: placeVal,
          killer: killer.value,
        },
        spoken,
        tooltip: () => {
          const tipRows: LogTooltipField[] = [
            { label: 'Status', value: statusVal },
            { label: 'Country', value: country.value },
            { label: 'Template', value: templateVal },
            { label: placeAtEvent ? 'Location' : 'Last seen at', value: placeVal },
          ];
          if (readHour !== eventHour) {
            tipRows.push({ label: 'Recorded', value: knownValue(formatDateTime(readHour)), tone: 'muted' });
          }
          if (afterHour !== null) {
            tipRows.push({ label: 'First appeared after', value: knownValue(formatDateTime(afterHour)), tone: 'muted' });
          }
          if (kind !== 'created') {
            tipRows.push({ label: 'Killer', value: killer.value });
          }
          return {
            title: nameSpoken,
            icon: icon ?? undefined,
            dateLine: `${by ? 'By ' : ''}${formatDateTime(eventHour)}`,
            rows: tipRows,
          };
        },
      };
    }

    const facetPills: LogFacetPill[] = [];
    if (options.showTotals) {
      if (options.events !== 'lost') {
        const createdCount = totals.get('created');
        facetPills.push({
          key: 'created',
          label: 'Created',
          count: createdCount !== undefined ? knownValue(createdCount) : notRecorded('Created total not recorded'),
          tone: 'positive',
        });
      }
      if (options.events !== 'created') {
        const lostCount = totals.get('lost');
        const transferCount = totals.get('transfer');
        facetPills.push({
          key: 'lost',
          label: 'Lost / removed',
          count: options.lossesAvailable && lostCount !== undefined
            ? knownValue(lostCount)
            : notRecorded('Losses not in this recording'),
          tone: 'negative',
        });
        facetPills.push({
          key: 'transfer',
          label: 'Transferred / merged',
          count: options.lossesAvailable && transferCount !== undefined
            ? knownValue(transferCount)
            : notRecorded('Transfers not in this recording'),
          tone: 'accent',
        });
      }
    }

    const warnings: string[] = [];
    const seenWarn = new Set<string>();
    const addWarn = (msg: string) => {
      if (msg && !seenWarn.has(msg)) {
        seenWarn.add(msg);
        warnings.push(msg);
      }
    };
    for (const msg of options.manifestWarnings) addWarn(msg);
    for (const msg of rawTail.warnings) addWarn(msg);
    const vanished = totals.get('vanished') ?? 0;
    if (vanished > 0) {
      addWarn(`Division left the census without a removal witness; no loss inferred (${vanished})`);
    }

    const gaps = mergeGaps(rawTail.missing, options.manifestGaps, hour);

    return {
      hour,
      total,
      baseOrdinal,
      firstOrdinal,
      offset,
      layout: 'row',
      columns: COLUMNS,
      rows,
      gaps,
      facetPills,
      empty: { kind: 'empty', message: 'No recorded division events match these settings.' },
      warnings,
    };
  },
};

registerLogProjection(divisionLogProjection);
