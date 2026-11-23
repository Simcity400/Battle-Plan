import WidgetState from '../components/WidgetState';
import type { IconName } from '../components/Icons';

export default function NoMembersState({ reason }: { reason: string; icon?: IconName }) {
  return <WidgetState kind="no-members" message={reason} />;
}
