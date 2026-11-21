import type { ReactNode } from 'react';
import { Icon } from './Icons';
import type { IconName } from './Icons';

export default function EmptyState({ icon, title, children, busy = false, tone, role }: {
  icon: IconName; title: string; children?: ReactNode; busy?: boolean; tone?: 'error'; role?: 'status' | 'alert';
}) {
  return <div className={`empty-state${tone ? ` is-${tone}` : ''}`} role={role}>
    <div className="empty-state-icon">{busy ? <span className="spinner" /> : <Icon name={icon} />}</div>
    <h2 className="empty-state-title">{title}</h2>
    {children}
  </div>;
}
