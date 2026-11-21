import type { CSSProperties } from 'react';
import type { Backdrop as BackdropSettings } from './settings';

export default function Backdrop({ backdrop, image, className }: { backdrop: BackdropSettings; image: string | null; className?: string }) {
  return <div className={className ? `app-backdrop ${className}` : 'app-backdrop'} data-kind={backdrop.kind} aria-hidden="true"
    style={{ '--backdrop-dim': `${backdrop.dim}%`, '--backdrop-blur': `${backdrop.blur}px` } as CSSProperties}>
    {backdrop.kind === 'image' && image && <>
      <div className="app-backdrop-image" style={{ backgroundImage: `url("${image}")` }} />
      <div className="app-backdrop-dim" />
    </>}
  </div>;
}
