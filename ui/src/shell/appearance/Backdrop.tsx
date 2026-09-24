import type { BackdropKind } from './settings';

/** The layer behind the whole interface: an ambient glow, a solid canvas or an imported image. */
export default function Backdrop({ kind, image }: { kind: BackdropKind; image: string | null }) {
  return <div className="app-backdrop" aria-hidden="true">
    {kind === 'image' && image && <>
      <div className="app-backdrop-image" style={{ backgroundImage: `url("${image}")` }} />
      <div className="app-backdrop-dim" />
    </>}
  </div>;
}
