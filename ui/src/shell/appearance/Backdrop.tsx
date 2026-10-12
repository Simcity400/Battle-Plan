import type { CSSProperties } from 'react';
import type { Backdrop as BackdropSettings } from './settings';

/** A backdrop layer: an ambient glow, a solid canvas or an imported image. The window's sits
 * behind the whole interface; the main view's own sits behind its panels only. */
export default function Backdrop({ backdrop, image, className }: { backdrop: BackdropSettings; image: string | null; className?: string }) {
  return <div className={className ? `app-backdrop ${className}` : 'app-backdrop'} data-kind={backdrop.kind} aria-hidden="true"
    style={{ '--backdrop-dim': `${backdrop.dim}%`, '--backdrop-blur': `${backdrop.blur}px` } as CSSProperties}>
    {backdrop.kind === 'image' && image && <>
      <div className="app-backdrop-image" style={{ backgroundImage: `url("${image}")` }} />
      <div className="app-backdrop-dim" />
    </>}
  </div>;
}
