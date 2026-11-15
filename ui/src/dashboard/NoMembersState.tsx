import WidgetState from '../components/WidgetState';
import type { IconName } from '../components/Icons';

/** What a widget with no group member to ask for shows (`noMembersAt`): its dated reason. */
export default function NoMembersState({ reason, icon }: { reason: string; icon?: IconName }) {
  return <WidgetState kind="empty" icon={icon} message={reason} />;
}
