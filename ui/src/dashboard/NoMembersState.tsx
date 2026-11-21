import WidgetState from '../components/WidgetState';
import type { IconName } from '../components/Icons';

export default function NoMembersState({ reason, icon }: { reason: string; icon?: IconName }) {
  return <WidgetState kind="empty" icon={icon} message={reason} />;
}
