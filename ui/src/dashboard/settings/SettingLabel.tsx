import type { ReactNode } from 'react';
import { Icon } from '../../components/Icons';

export function SettingLabel({ label, hint }: { label: ReactNode; hint?: string }) {
  return <span className="setting-label" data-tooltip={hint || undefined}>
    {label}{hint && <><Icon name="info" className="setting-label-info" /><span className="sr-only">{hint}</span></>}
  </span>;
}
