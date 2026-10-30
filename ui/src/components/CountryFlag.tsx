import { useEffect, useRef, useState } from 'react';
import type { CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { FAILED_FACT } from '../dashboard/countryFacts';
import { flagReadKey, useFlagReads } from '../dashboard/countryFlags';
import { countryFlagUnavailable, fitCountryFlag } from './countryFlagPresentation';
export { countryFlagUnavailable } from './countryFlagPresentation';
export type { CountryFlagLook } from './countryFlagPresentation';

/** A flag to draw: its picture's URL, as a flag reply carries it, or straight-alpha RGBA, as the
 * recorder's status carries it. */
export type FlagImage = { width: number; height: number } & ({ png: string } | { pixels: ArrayLike<number> });

/** Content flags are small (82x52 in vanilla): each is drawn from its picture at its shown
 * size and screen density with smoothing, so a stretched flag blends rather than shows its
 * pixels. One observer serves every flag shown, after layout, when it appears and whenever it
 * resizes. */
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

/** Shows `image` on `element` from now on, until the returned stop: the flag it showed is
 * cleared at once, so a picture that cannot be decoded leaves an empty frame, never the flag
 * it replaces. */
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
    // Decoded off the main thread, the flag is drawn once its picture is ready.
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

/** Dated row flags retain loading and unavailable reasons in the shared presentation. */
export function CountryFlagUrl({ url, reason, pending, className = '' }: { url?: string; reason?: string; pending?: boolean; className?: string }) {
  const [failed, setFailed] = useState<string>();
  if (url && failed !== url) return <img className={`country-flag-art ${className}`} src={url} alt="" onError={() => setFailed(url)} />;
  const { text: tooltip } = countryFlagUnavailable(reason, pending, !!url && failed === url);
  return <span className={`country-flag country-flag-unavailable ${className}`} role="img" aria-label={tooltip} data-tooltip={tooltip} />;
}

/** A country's flag from a recording's own installed content, at the hour the country was observed, as
 * preparation chose it. Without a readable flag it shows an empty frame, never a substitute.
 * The default `panel` size is app styling: the native shield with its overlay's edge shading
 * evened out. */
export function SourceFlag({ client, source, tag, size = 'panel' }: { client: DashboardClient; source: DashboardSource; tag: string | null; size?: CountryFlagSize }) {
  const country = tag ? source.metadata.countries.find(item => item.tag === tag) : undefined;
  const branch = source.metadata.branches.find(item => item.id === country?.observedBranchId) ?? source.metadata.branches[0];
  const hour = country && country.observedBranchId === branch?.id ? country.observedHour : branch?.firstHour;
  const read = tag && branch && hour != null ? { tag, branchId: branch.id, hour } : null;
  const flags = useFlagReads(client, source, read ? [read] : [], true, size);
  const flag = read ? flags.get(flagReadKey(read)) : undefined;
  return <CountryFlag image={flag && flag !== FAILED_FACT ? flag.image : undefined} />;
}
