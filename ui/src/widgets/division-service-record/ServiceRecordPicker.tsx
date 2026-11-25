import { formatDay } from '../../dashboard/calendar.ts';
import { subjectToken } from '../../dashboard/chosen.ts';
import { useProduct } from '../../dashboard/products.ts';
import { productRequest } from '../../dashboard/request.ts';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork.ts';
import { occurrenceSelection } from '../../families/table/logHelpers.ts';
import type { PickerProps } from '../../families/table/picker.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ServiceChoicesReply } from '../../generated/dashboard';
import { nameOf, removals } from './serviceRecordProjection.ts';

const groups = ['Recently destroyed', 'Most decorated', 'Other removals', 'Last observed'];

export default function ServiceRecordPicker({ runtime, context, hour, selected, children }: PickerProps) {
  const { track } = context;
  const request = productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'division-service-record-choices', params: { branchId: track.branch.id, hour, ...occurrenceSelection(context) } });
  const { data, error } = useProduct<ServiceChoicesReply>(runtime.client, request, true, undefined, track.source);
  const choices = data?.choices ?? [];
  const sprite = useSpriteArtwork(runtime, track, choices.map(row => row.sprite ? { key: row.sprite, frame: 0 } : null), true);
  if (error || !data) return <div role="status">{error?.message ?? 'Loading divisions...'}</div>;
  const items = choices.map(row => ({
    value: subjectToken(track.source.metadata.campaignKey, row.division), label: nameOf(row), checked: row.division === selected,
    icon: row.sprite ? sprite({ key: row.sprite, frame: 0 }) ?? undefined : undefined, group: row.removedAt !== null ? ['division_destroyed', 'division_overrun'].includes(row.removed ?? '') ? 'Recently destroyed' : 'Other removals'
      : row.lastSeen < hour ? 'Last observed' : 'Most decorated',
    country: row.tag ?? 'Country not recorded', detail: row.template ?? 'Template not recorded',
    note: row.removedAt !== null ? `${removals[row.removed ?? ''] ?? 'Removed'} · ${formatDay(row.removedAt)}`
      : row.medals === null ? 'Honours not recorded' : `${formatScalar(row.medals)} ${row.medals === 1 ? 'honour' : 'honours'}`,
  })).sort((a, b) => groups.indexOf(a.group) - groups.indexOf(b.group));
  return children(items);
}
