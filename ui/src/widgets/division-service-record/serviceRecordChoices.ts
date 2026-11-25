import { formatDay } from '../../dashboard/calendar.ts';
import { subjectToken } from '../../dashboard/chosen.ts';
import { productRequest } from '../../dashboard/request.ts';
import { occurrenceSelection } from '../../families/table/logHelpers.ts';
import type { TablePicker } from '../../families/table/picker.ts';
import { knownValue, notRecorded } from '../../families/value.ts';
import { formatDateParts } from '../../families/valueFormat.ts';
import type { ServiceChoicesReply, ServiceDivision } from '../../generated/dashboard';

export const removals: Readonly<Record<string, string>> = {
  loss: 'Lost', division_destroyed: 'Destroyed', division_overrun: 'Overrun', division_deleted: 'Removed', division_disbanded: 'Disbanded',
  division_capitulation: 'Removed at capitulation', division_scripted: 'Removed by script', division_template_removed: 'Template removed',
  returned: 'Returned', merged: 'Merged', hq_transferred: 'HQ transferred', volunteer_transferred: 'Volunteer transferred', exile_transferred: 'Exile transferred',
};
export const nameOf = (row: ServiceDivision) => row.name ?? `Division name not recorded (${row.division})`;
const groups = ['Recently destroyed', 'Most decorated', 'Other removals', 'Last observed'];
const DESTROYED = new Set(['division_destroyed', 'division_overrun']);

function removedOn(at: number, hour: number): string {
  const parts = formatDateParts(at);
  if (!parts) return formatDay(at);
  return parts.year === formatDateParts(hour)?.year ? parts.dayMonth : parts.full;
}

export const serviceRecordPicker: TablePicker = {
  request(context, hour) {
    const { source, branch } = context.track;
    return productRequest({ sourceId: source.id, revision: source.revision, product: 'division-service-record-choices',
      params: { branchId: branch.id, hour, ...occurrenceSelection(context) } });
  },
  items(data, { track }, hour, selected) {
    const { choices } = data as ServiceChoicesReply;
    return choices.map(row => ({
      value: subjectToken(track.source.metadata.campaignKey, row.division), label: nameOf(row), checked: row.division === selected,
      ...(row.sprite ? { sprite: row.sprite } : {}), ...(row.tag ? { country: row.tag } : {}), ...(row.template ? { detail: row.template } : { detailMissing: 'Template not recorded' }),
      group: row.removedAt !== null ? DESTROYED.has(row.removed ?? '') ? 'Recently destroyed' : 'Other removals'
        : row.lastSeen < hour ? 'Last observed' : 'Most decorated',
      ...(row.removedAt !== null
        ? { state: { kind: 'ended' as const, label: `${removals[row.removed ?? ''] ?? 'Removed'} ${removedOn(row.removedAt, hour)}` } }
        : { quantity: { value: row.medals === null ? notRecorded('Honours not recorded') : knownValue(row.medals), unit: row.medals === 1 ? 'honour' : 'honours' } }),
    })).sort((a, b) => groups.indexOf(a.group) - groups.indexOf(b.group));
  },
};
