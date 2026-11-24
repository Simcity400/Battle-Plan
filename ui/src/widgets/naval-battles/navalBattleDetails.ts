import { type Failure, failureText } from '../../dashboard/queryError.ts';
import type { TooltipRow } from '../../charts/tooltip';
import { formatDateTime, isEngineHour } from '../../dashboard/calendar.ts';
import { int, object } from '../../dashboard/validation.ts';
import type { NavalBattleDetail, NavalBattleDetailsReply } from '../../generated/dashboard';

const text = (v: unknown): v is string | null => v === null || typeof v === 'string';
const hour = (v: unknown): v is number => int(v) && isEngineHour(v);
function detail(v: unknown): v is NavalBattleDetail {
  if (!object(v) || typeof v.id !== 'string' || v.id.length === 0 || !hour(v.hour)) return false;
  const readHour = v.hour;
  return (v.rosterHour === null || hour(v.rosterHour) && v.rosterHour <= readHour)
    && (v.shipRead === null || hour(v.shipRead) && v.shipRead <= readHour) && text(v.unavailable)
    && Array.isArray(v.fleets) && v.fleets.length === 2 && v.fleets.every(fleet => object(fleet)
      && Array.isArray(fleet.names) && fleet.names.every(name => typeof name === 'string' && name.length > 0) && text(fleet.unknown))
    && Array.isArray(v.namedShips) && v.namedShips.every(ship => object(ship) && hour(ship.sinkHour)
      && hour(ship.knownAt) && ship.sinkHour <= ship.knownAt && ship.knownAt <= readHour && text(ship.name) && text(ship.className));
}
export function validateNavalBattleDetails(v: unknown): NavalBattleDetailsReply {
  if (!object(v) || !Array.isArray(v.rows) || !v.rows.every(detail)
    || new Set(v.rows.map(row => JSON.stringify([row.id, row.hour]))).size !== v.rows.length
    || !Array.isArray(v.warnings) || !v.warnings.every(warning => typeof warning === 'string')) {
    throw new Error('The naval battle details reply is malformed.');
  }
  return v as unknown as NavalBattleDetailsReply;
}
export const navalDetailKey = (id: string, hour: number) => JSON.stringify([id, hour]);
export function navalFleetDescription(detail: NavalBattleDetail | undefined, read: { pending: boolean; error?: Failure | undefined }): string {
  if (!detail) return read.error ? failureText(read.error, 'Fleets couldn’t load') : read.pending ? 'Loading fleets…' : 'Fleets not read for this battle';
  return detail.fleets.map(side => side.names.length ? side.names.join(', ') + (side.unknown ? ' + unavailable' : '')
    : side.unknown ? 'Fleet names not recorded' : 'No fleets').join(' v ');
}
export function navalDetailTooltip(detail: NavalBattleDetail | undefined, error?: Failure): TooltipRow[] {
  if (!detail) return [{ detail: error?.message, label: error ? failureText(error, 'Couldn’t load battle details.') : 'Loading battle details…', tone: 'muted', divider: true }];
  const rows: TooltipRow[] = [];
  for (const [index, side] of detail.fleets.entries()) {
    rows.push({ label: `${index === 0 ? 'Attacking' : 'Defending'} fleets`, strong: true, divider: true });
    rows.push(...side.names.map(label => ({ label, indent: true })));
    if (side.unknown) rows.push({ label: 'Fleet details unavailable.', indent: true, tone: 'muted' });
    if (!side.names.length && !side.unknown) rows.push({ label: 'No fleets', indent: true });
  }
  rows.push({ label: 'Named ships lost', strong: true, divider: true });
  if (detail.unavailable || detail.shipRead === null) rows.push({ label: 'Named ship losses unavailable.', tone: 'muted' });
  else {
    if (!detail.namedShips.length) rows.push({ label: 'No named ship losses recorded.', tone: 'muted' });
    for (const ship of detail.namedShips) {
      rows.push({ label: ship.name ?? 'Ship name unavailable', value: ship.className ?? 'Class unavailable', indent: true });
      rows.push({ label: 'Sunk', value: formatDateTime(ship.sinkHour), indent: true });
    }
  }
  return rows;
}
