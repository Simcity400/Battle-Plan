import type { ReactNode } from 'react';
import { Icon } from '../../components/Icons';

/** A setting's name. Its explanation shows on hover and is read by screen readers, so
 * every widget's settings panel stays one compact line per setting. */
export function SettingLabel({ label, hint }: { label: ReactNode; hint?: string }) {
  return <span className="setting-label" data-tooltip={hint || undefined}>
    {label}{hint && <><Icon name="info" className="setting-label-info" /><span className="sr-only">{hint}</span></>}
  </span>;
}
