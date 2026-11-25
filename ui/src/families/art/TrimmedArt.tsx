import { useEffect, useState } from 'react';
import { artDrawnBounds } from '../../draw/kit.ts';

const insets = new Map<string, string | null>();
const measuring = new Map<string, Promise<string | null>>();

function measure(src: string): Promise<string | null> {
  let held = measuring.get(src);
  if (held) return held;
  held = (async () => {
    try {
      const bitmap = await createImageBitmap(await (await fetch(src)).blob());
      const box = artDrawnBounds(bitmap, bitmap.width, bitmap.height);
      const inset = box ? `inset(${box.y}px ${bitmap.width - box.x - box.width}px ${bitmap.height - box.y - box.height}px ${box.x}px)` : null;
      bitmap.close();
      return inset;
    } catch {
      return null;
    }
  })().then(inset => {
    insets.set(src, inset);
    measuring.delete(src);
    return inset;
  });
  measuring.set(src, held);
  return held;
}

export default function TrimmedArt({ src, className }: { src: string; className?: string }) {
  const [shown, setShown] = useState<{ src: string; inset: string | null } | null>(() => insets.has(src) ? { src, inset: insets.get(src)! } : null);
  useEffect(() => {
    if (insets.has(src)) { setShown({ src, inset: insets.get(src)! }); return; }
    let live = true;
    void measure(src).then(inset => { if (live) setShown({ src, inset }); });
    return () => { live = false; };
  }, [src]);
  const inset = shown?.src === src ? shown.inset : undefined;
  return <img className={className} src={src} alt="" style={inset === undefined ? { visibility: 'hidden' } : inset ? { objectViewBox: inset } : undefined} />;
}
