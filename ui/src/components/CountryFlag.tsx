import { useEffect, useRef, useState } from 'react';
import type { CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { isFailedFact } from '../dashboard/countryFacts.ts';
import { flagReadKey, useFlagReads } from '../dashboard/countryFlags';
import { countryFlagUnavailable, fitCountryFlag } from './countryFlagPresentation';
export { countryFlagUnavailable } from './countryFlagPresentation';
export type { CountryFlagLook } from './countryFlagPresentation';

export type FlagImage = { width: number; height: number } & ({ png: string } | { pixels: ArrayLike<number> });

const shown = new WeakMap<Element, { picture: CanvasImageSource; width: number; height: number }>();
let resized: ResizeObserver | undefined;
function drawFlags(entries: readonly ResizeObserverEntry[]) {
  for (const { target } of entries) {
    const source = shown.get(target);
    if (!source || !(target instanceof HTMLCanvasElement)) continue;
    const { width, height } = target.getBoundingClientRect();
    target.width = Math.max(1, Math.round(width * devicePixelRatio)) || source.width;
    target.height = Math.max(1, Math.round(height * devicePixelRatio)) || source.height;
    const context = target.getContext('2d');
    if (!context) continue;
    context.imageSmoothingQuality = 'high';
    const fitted = fitCountryFlag(source.width, source.height, target.width, target.height);
    context.drawImage(source.picture, fitted.x, fitted.y, fitted.width, fitted.height);
  }
}

export function showFlag(element: HTMLCanvasElement, image: FlagImage): () => void {
  let live = true;
  element.getContext('2d')?.clearRect(0, 0, element.width, element.height);
  const show = (picture: CanvasImageSource) => {
    if (!live) return;
    shown.set(element, { picture, width: image.width, height: image.height });
    resized ??= new ResizeObserver(drawFlags);
    resized.observe(element);
  };
  if ('png' in image) {
    const picture = new Image();
    picture.src = image.png;
    void picture.decode().then(() => show(picture), () => {});
  } else {
    const pixels = image.pixels instanceof Uint8ClampedArray ? image.pixels : Uint8ClampedArray.from(image.pixels);
    const picture = document.createElement('canvas');
    picture.width = image.width;
    picture.height = image.height;
    picture.getContext('2d')?.putImageData(new ImageData(pixels as Uint8ClampedArray<ArrayBuffer>, image.width, image.height), 0, 0);
    show(picture);
  }
  return () => { live = false; resized?.unobserve(element); shown.delete(element); };
}

export function CountryFlag({ image }: { image?: FlagImage }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const element = canvas.current;
    if (!element || !image) return;
    return showFlag(element, image);
  }, [image]);
  return image ? <canvas ref={canvas} className="country-flag country-flag-art" width={image.width} height={image.height} aria-hidden="true" />
    : <span className="country-flag country-flag-unavailable" aria-hidden="true" />;
}

export function CountryFlagUrl({ url, reason, detail, pending, className = '' }: {
  url?: string; reason?: string; detail?: string; pending?: boolean; className?: string;
  fixedSize?: boolean;
}) {
  const [failed, setFailed] = useState<string>();
  if (url && failed !== url) return <img className={`country-flag-art ${className}`} src={url} alt="" onError={() => setFailed(url)} />;
  const { text: tooltip } = countryFlagUnavailable(reason, pending, !!url && failed === url);
  return <span className={`country-flag country-flag-unavailable ${className}`} role="img" data-detail={detail} aria-label={tooltip} data-tooltip={tooltip} />;
}


export function SourceFlag({ client, source, tag, size = 'panel' }: { client: DashboardClient; source: DashboardSource; tag: string | null; size?: CountryFlagSize }) {
  const country = tag ? source.metadata.countries.find(item => item.tag === tag) : undefined;
  const branch = source.metadata.branches.find(item => item.id === country?.observedBranchId) ?? source.metadata.branches[0];
  const hour = country && country.observedBranchId === branch?.id ? country.observedHour : branch?.firstHour;
  const read = tag && branch && hour != null ? { tag, branchId: branch.id, hour } : null;
  const flags = useFlagReads(client, source, read ? [read] : [], true, size);
  const flag = read ? flags.get(flagReadKey(read)) : undefined;
  return <CountryFlag image={flag && !isFailedFact(flag) ? flag.image : undefined} />;
}
