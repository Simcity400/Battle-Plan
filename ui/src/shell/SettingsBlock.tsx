import { useId } from 'react';
import type { ReactNode } from 'react';

export function SettingsBlock({ id, title, description, actions, children, plain = false }: { id?: string; title: string; description?: string; actions?: ReactNode; children: ReactNode; plain?: boolean }) {
  return <section className={`settings-block${plain ? ' settings-block-plain' : ''}`} id={id} aria-label={title}>
    <div className="settings-block-heading"><div><h3>{title}</h3>{description && <p>{description}</p>}</div>{actions && <div className="settings-block-actions">{actions}</div>}</div>
    <div className="settings-group">{children}</div>
  </section>;
}

export function ConfirmRow({ title, detail, action, onCancel, onConfirm }: { title: string; detail: string; action: string; onCancel: () => void; onConfirm: () => void }) {
  const id = useId();
  return <div className="settings-row settings-confirm" role="alertdialog" aria-labelledby={`${id}-title`} aria-describedby={`${id}-detail`}
    onKeyDown={event => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onCancel(); } }}>
    <span className="settings-row-text"><strong id={`${id}-title`}>{title}</strong><small id={`${id}-detail`}>{detail}</small></span>
    <span className="settings-confirm-actions">
      <button type="button" className="button small quiet" autoFocus onClick={onCancel}>Cancel</button>
      <button type="button" className="button small is-danger" onClick={onConfirm}>{action}</button>
    </span>
  </div>;
}
