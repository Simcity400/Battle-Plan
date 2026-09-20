import { useEffect, useRef } from 'react';
import type { CountryFlagImage } from './client';

export function CountryFlag({ image }: { image?: CountryFlagImage }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const context = canvas.current?.getContext('2d');
    if (!context || !image) return;
    context.putImageData(new ImageData(new Uint8ClampedArray(image.pixels), image.width, image.height), 0, 0);
  }, [image]);
  return image ? <canvas ref={canvas} className="country-flag" width={image.width} height={image.height} aria-hidden="true" />
    : <span className="country-flag country-flag-unavailable" aria-hidden="true" />;
}
