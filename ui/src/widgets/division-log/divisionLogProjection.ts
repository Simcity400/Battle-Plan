import { formatDateTime } from '../../dashboard/calendar.ts';
import type { MissingSpan } from '../../generated/dashboard';
import type { FamilySources } from '../../families/contract.ts';
import { logLookAt, type LogLooksMap, type LogProjection } from '../../families/log/declaration.ts';
import type { LogColumnTrack, LogFacetPill, LogModel, LogRowSpec, LogTextPart, LogTooltipField } from '../../families/log/model.ts';
import { registerLogProjection } from '../../families/log/registry.ts';
import {
  cellAt,
  cellNumber,
  cellText,
  columnIndex,
  facetTotalMap,
  isLogTailReply,
  newestFirstWireRows,
  nonEmptyText,
  parseOrdinal,
} from '../../families/log/tailReply.ts';
import { LogRowProjectionCache } from '../../families/log/rowCache.ts';
import { knownValue, notApplicable, notRecorded, partialValue, type Value } from '../../families/value.ts';
import { divisionKindFromNumber, divisionStatus } from './divisionLogData.ts';

export const DIVISION_LOG_PROJECTION_KIND = 'division-log';
const rowCache = new LogRowProjectionCache();
const REMOVAL_GAP_FRAGMENT = 'division equipment deletions';

export interface DivisionLogProjectionOptions {
  readonly events: string;
  readonly showTotals: boolean;
  readonly lossesAvailable: boolean;
  readonly fromHour?: number;
  readonly looks: LogLooksMap;
  readonly manifestWarnings: readonly string[];
  readonly manifestGaps: readonly MissingSpan[];
}

const COLUMNS: readonly LogColumnTrack[] = [
  { id: 'date', width: 58 },
  { id: 'art', width: 24 },
  { id: 'body', flex: 1, minWidth: 180 },
  { id: 'status', width: 78, align: 'end', priority: 2 },
];

function countryValue(
  looks: LogLooksMap,
  tag: string | null,
  hour: number,
  missingReason: string,
): { readonly value: Value<string>; readonly tagValue: Value<string>; readonly look: ReturnType<typeof logLookAt>; readonly spoken: string } {
  if (!tag) {
    return {
      value: notRecorded(missingReason),
      tagValue: notRecorded(missingReason),
      look: null,
      spoken: missingReason,
    };
  }
  const look = logLookAt(looks, tag, hour);
  const resolved = nonEmptyText(look?.name);
  if (resolved) {
    return { value: knownValue(resolved), tagValue: knownValue(tag), look, spoken: resolved };
  }
  const reason = look?.status ?? 'Country display name not recorded';
  return {
    value: notRecorded(reason),
    tagValue: knownValue(tag),
    look,
    spoken: `${reason} (${tag})`,
  };
}

function mergeGaps(a: readonly MissingSpan[], b: readonly MissingSpan[], fromHour: number, upToHour: number): MissingSpan[] {
  const out: MissingSpan[] = [];
  const seen = new Set<string>();
  for (const list of [a, b]) {
    for (const span of list) {
      if (span.firstHour > upToHour || span.lastHour < fromHour) continue;
      const firstHour = Math.max(span.firstHour, fromHour);
      const lastHour = Math.min(span.lastHour, upToHour);
      const key = `${firstHour}:${lastHour}:${span.reason}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ firstHour, lastHour, reason: span.reason });
    }
  }
  out.sort((x, y) => x.firstHour - y.firstHour || x.lastHour - y.lastHour);
  return out;
}

function isRemovalOnlyGap(span: MissingSpan): boolean {
  return span.reason.toLowerCase().includes(REMOVAL_GAP_FRAGMENT);
}

function intervalCoverage(
  fromHour: number,
  toHour: number,
  spans: readonly MissingSpan[],
): { readonly kind: 'complete' } | { readonly kind: 'none'; readonly reason: string } | { readonly kind: 'partial'; readonly reason: string } {
  if (fromHour > toHour || spans.length === 0) return { kind: 'complete' };
  const clipped: { first: number; last: number; reason: string }[] = [];
  for (const span of spans) {
    const first = Math.max(span.firstHour, fromHour);
    const last = Math.min(span.lastHour, toHour);
    if (first <= last) clipped.push({ first, last, reason: span.reason });
  }
  if (clipped.length === 0) return { kind: 'complete' };
  clipped.sort((a, b) => a.first - b.first || a.last - b.last);
  const firstReason = clipped[0]!.reason;
  let missingHours = 0;
  let curFirst = clipped[0]!.first;
  let curLast = clipped[0]!.last;
  for (let i = 1; i < clipped.length; i++) {
    const next = clipped[i]!;
    if (next.first <= curLast + 1) {
      if (next.last > curLast) curLast = next.last;
    } else {
      missingHours += curLast - curFirst + 1;
      curFirst = next.first;
      curLast = next.last;
    }
  }
  missingHours += curLast - curFirst + 1;
  const totalHours = toHour - fromHour + 1;
  if (missingHours >= totalHours) return { kind: 'none', reason: firstReason };
  return { kind: 'partial', reason: firstReason };
}

function pillCountForInterval(
  rawCount: number | undefined,
  available: boolean,
  fallbackUnavailable: string,
  fromHour: number | undefined,
  toHour: number,
  relevantGaps: readonly MissingSpan[],
): Value<number> {
  if (fromHour !== undefined) {
    const cov = intervalCoverage(fromHour, toHour, relevantGaps);
    if (cov.kind === 'none') return notRecorded(cov.reason);
    if (cov.kind === 'partial') {
      if (rawCount === undefined) return notRecorded(cov.reason);
      return partialValue(rawCount, cov.reason, { atLeast: true });
    }
  }
  if (!available) return notRecorded(fallbackUnavailable);
  if (rawCount === undefined) return notRecorded(fallbackUnavailable);
  return knownValue(rawCount);
}

export const divisionLogProjection: LogProjection<DivisionLogProjectionOptions> = {
  family: 'log',
  kind: DIVISION_LOG_PROJECTION_KIND,
  layout: 'row',
  nextChange(_sources: FamilySources, hour: number, options: DivisionLogProjectionOptions): number {
    let soonest = Infinity;
    for (const span of options.manifestGaps) {
      if (span.firstHour > hour) {
        if (span.firstHour < soonest) soonest = span.firstHour;
      } else if (span.firstHour <= hour && hour < span.lastHour) {
        const next = hour + 1;
        if (next < soonest) soonest = next;
      } else if (hour === span.lastHour) {
        const exit = span.lastHour + 1;
        if (exit < soonest) soonest = exit;
      }
    }
    return soonest;
  },
  project(sources: FamilySources, hour: number, options: DivisionLogProjectionOptions): LogModel {
    const rawTail = sources.panels.get('tail');
    const offsetRaw = sources.panels.get('__offset');
    const offset = typeof offsetRaw === 'number' && Number.isFinite(offsetRaw) ? offsetRaw : 0;
    const fromBound = options.fromHour ?? -Infinity;

    if (!isLogTailReply(rawTail)) {
      return {
        hour,
        ...(options.fromHour !== undefined ? { fromHour: options.fromHour } : {}),
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

    const orderedRows = newestFirstWireRows(rawTail.rows);
    const rows: LogRowSpec[] = new Array(orderedRows.length);
    for (let i = 0; i < orderedRows.length; i++) {
      const wire = orderedRows[i]!;
      rows[i] = rowCache.getOrProject(options.looks, wire, () => {
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
          ? {
            value: notApplicable('Division creation has no attacker') as Value<string>,
            tagValue: notApplicable('Division creation has no attacker') as Value<string>,
            look: null,
            spoken: 'Not applicable',
          }
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
            ? placeMissing
              ? partialValue(placeName, placeMissing)
              : knownValue(placeName)
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
          { text: knownValue(': '), weight: 'bold' },
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
          : placeName
            ? placeMissing ? `${placeName} (partial: ${placeMissing})` : placeName
            : placeMissing ?? 'Unnamed province';
        const statusSpoken = statusText ?? 'Status not recorded';
        const spoken = `${country.spoken}: ${nameSpoken}, ${statusSpoken}, ${templateSpoken}, ${placeSpoken}${killerTag ? `, by ${killer.spoken}` : ''}.`;

        return {
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
            { kind: 'text', text: statusVal, tone: 'muted' },
          ],
          rawMagnitudes: {
            kind: kindVal,
            countryTag: country.tagValue,
            killerTag: killer.tagValue,
          },
          spoken,
          tooltip: () => {
            const tipRows: LogTooltipField[] = [
              { label: 'Status', value: statusVal },
              { label: 'Country', value: country.value },
            ];
            if (tag && country.value.kind !== 'known') {
              tipRows.push({ label: 'Tag', value: knownValue(tag), tone: 'muted' });
            }
            tipRows.push(
              { label: 'Template', value: templateVal },
              { label: placeAtEvent ? 'Location' : 'Last seen at', value: placeVal },
            );
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
              ...(by || eventHour !== readHour ? { subtitle: `${by ? 'By ' : ''}${formatDateTime(eventHour)}` } : {}),
              rows: tipRows,
            };
          },
        };
      });
    }

    const gaps = mergeGaps(rawTail.missing, options.manifestGaps, fromBound, hour);
    const createdGaps = gaps.filter(span => !isRemovalOnlyGap(span));

    const facetPills: LogFacetPill[] = [];
    if (options.showTotals) {
      if (options.events !== 'lost') {
        facetPills.push({
          key: 'created',
          label: 'Created',
          count: pillCountForInterval(
            totals.get('created'),
            true,
            'Created total not recorded',
            options.fromHour,
            hour,
            createdGaps,
          ),
          tone: 'positive',
        });
      }
      if (options.events !== 'created') {
        facetPills.push({
          key: 'lost',
          label: 'Lost / removed',
          count: pillCountForInterval(
            totals.get('lost'),
            options.lossesAvailable,
            'Losses not in this recording',
            options.fromHour,
            hour,
            gaps,
          ),
          tone: 'negative',
        });
        facetPills.push({
          key: 'transfer',
          label: 'Transferred / merged',
          count: pillCountForInterval(
            totals.get('transfer'),
            options.lossesAvailable,
            'Transfers not in this recording',
            options.fromHour,
            hour,
            gaps,
          ),
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

    return {
      hour,
      ...(options.fromHour !== undefined ? { fromHour: options.fromHour } : {}),
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
