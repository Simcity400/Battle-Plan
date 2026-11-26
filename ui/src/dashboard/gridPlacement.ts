import { useLayoutEffect, useMemo, useState } from 'react';
import { setTransform } from 'react-grid-layout';
import type { Position, ResponsiveProps } from 'react-grid-layout';
import { deviceScale, snapToDevice, watchDevicePixelRatio, windowRatio } from '../charts/surface.ts';

export type GridPlacement = NonNullable<ResponsiveProps['positionStrategy']>;

function devicePixelPlacement(scale: number): GridPlacement {
  return {
    type: 'transform',
    scale: 1,
    calcStyle: (pos: Position) => setTransform({ ...pos, left: snapToDevice(pos.left, scale), top: snapToDevice(pos.top, scale) }),
  };
}

export function useGridPlacement(container: { readonly current: HTMLElement | null }): GridPlacement {
  const [scale, setScale] = useState(windowRatio);
  useLayoutEffect(() => {
    const read = () => setScale(container.current ? deviceScale(container.current) : windowRatio());
    read();
    return watchDevicePixelRatio(read);
  }, [container]);
  return useMemo(() => devicePixelPlacement(scale), [scale]);
}
