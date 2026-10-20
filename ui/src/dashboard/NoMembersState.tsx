import WidgetState from '../components/WidgetState';
import type { IconName } from '../components/Icons';
import { MEMBERSHIP_PREPARING } from './groups';

/** What a widget with no group member to ask for shows (`noMembersAt`): its dated reason,
 * or loading while the hour is still being prepared. */
export default function NoMembersState({ reason, icon }: { reason: string | null; icon?: IconName }) {
  return reason ? <WidgetState kind="empty" icon={icon} message={reason} /> : <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
}
