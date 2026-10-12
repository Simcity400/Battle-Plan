import { useEffect, useRef, useState } from 'react';
import type { CountryFlagImage, CountryFlagReply, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { loadProduct } from '../dashboard/products';
import { countryFlagUnavailable, fitCountryFlag } from './countryFlagPresentation';
export { countryFlagUnavailable } from './countryFlagPresentation';
export type { CountryFlagLook } from './countryFlagPresentation';

/** Straight-alpha RGBA, as a product reply carries it or as the flag store keeps it. */
export interface FlagPixels { width: number; height: number; pixels: ArrayLike<number> }

/** Content flags are small (82x52 in vanilla): each is drawn from its pixels' canvas at
 * its shown size and screen density with smoothing, so a stretched flag blends rather
 * than shows its pixels. One observer serves every flag shown, after layout, when it
 * appears and whenever it resizes. */
const shown = new WeakMap<Element, HTMLCanvasElement>();
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
    context.drawImage(source, fitted.x, fitted.y, fitted.width, fitted.height);
  }
}

export function CountryFlag({ image }: { image?: FlagPixels }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const element = canvas.current;
    if (!element || !image) return;
    const pixels = image.pixels instanceof Uint8ClampedArray ? image.pixels : Uint8ClampedArray.from(image.pixels);
    const source = document.createElement('canvas');
    source.width = image.width;
    source.height = image.height;
    source.getContext('2d')?.putImageData(new ImageData(pixels as Uint8ClampedArray<ArrayBuffer>, image.width, image.height), 0, 0);
    shown.set(element, source);
    resized ??= new ResizeObserver(drawFlags);
    resized.observe(element);
    return () => { resized?.unobserve(element); shown.delete(element); };
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

/** A country's flag from a recording's own installed content, at the hour the country
 * was observed. Without a readable flag it shows an empty frame, never a substitute. */
export function SourceFlag({ client, source, tag, size = 'panel' }: { client: DashboardClient; source: DashboardSource; tag: string | null; size?: CountryFlagSize }) {
  const country = tag ? source.metadata.countries.find(item => item.tag === tag) : undefined;
  const branch = source.metadata.branches.find(item => item.id === country?.observedBranchId) ?? source.metadata.branches[0];
  const hour = country && country.observedBranchId === branch?.id ? country.observedHour : branch?.firstHour;
  const key = `${source.id}:${source.revision}:${tag}:${branch?.id}:${hour}:${size}`;
  const [flag, setFlag] = useState<{ key: string; image?: CountryFlagImage }>({ key: '' });
  useEffect(() => {
    if (!tag || !branch || hour == null) return;
    const controller = new AbortController();
    void loadProduct<CountryFlagReply>(client, { sourceId: source.id, revision: source.revision, product: 'country-flags', params: { branchId: branch.id, hour, tags: [tag], size } }, controller.signal)
      .then(reply => { if (!controller.signal.aborted) setFlag({ key, image: reply.flags.find(image => image.tag === tag) }); },
        () => { if (!controller.signal.aborted) setFlag({ key }); });
    return () => controller.abort();
  }, [client, source.id, source.revision, tag, branch?.id, hour, size, key]);
  return <CountryFlag image={flag.key === key ? flag.image : undefined} />;
}
