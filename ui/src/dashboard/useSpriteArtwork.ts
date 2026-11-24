import type { WidgetRuntime } from './definitions';
import type { BoundTrack } from './model';
import { useArtResolutions } from '../families/art/react.ts';
import { spriteFrames, spriteHandle, type SpriteFrame } from '../families/art/transport.ts';

export function useSpriteArtwork(
  runtime: WidgetRuntime,
  track: BoundTrack | undefined,
  sprites: Iterable<SpriteFrame | null | undefined>,
  active: boolean,
): (sprite: SpriteFrame | null | undefined) => string | null {
  const resolve = useArtResolutions(runtime, track, spriteFrames(sprites).map(spriteHandle), active);
  return sprite => {
    if (!sprite) return null;
    const art = resolve(spriteHandle(sprite));
    return art.status === 'ready' ? art.url : null;
  };
}
