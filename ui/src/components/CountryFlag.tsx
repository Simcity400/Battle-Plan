import { useEffect, useRef, useState } from 'react';
import type { CountryFlagImage, CountryFlagReply, CountryFlagSize } from '../generated/dashboard';
import type { DashboardClient, DashboardSource } from '../dashboard/client';
import { loadProduct } from '../dashboard/products';

/** Straight-alpha RGBA, as a product reply carries it or as the flag store keeps it. */
export interface FlagPixels { width: number; height: number; pixels: ArrayLike<number> }

export function CountryFlag({ image }: { image?: FlagPixels }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const context = canvas.current?.getContext('2d');
    if (!context || !image) return;
    const pixels = image.pixels instanceof Uint8ClampedArray ? image.pixels : Uint8ClampedArray.from(image.pixels);
    context.putImageData(new ImageData(pixels as Uint8ClampedArray<ArrayBuffer>, image.width, image.height), 0, 0);
  }, [image]);
  return image ? <canvas ref={canvas} className="country-flag" width={image.width} height={image.height} aria-hidden="true" />
    : <span className="country-flag country-flag-unavailable" aria-hidden="true" />;
}

/** A country's flag from a recording's own installed content, at the hour the country
 * was observed. Without a readable flag it shows an empty frame, never a substitute. */
export function SourceFlag({ client, source, tag, size = 'medium' }: { client: DashboardClient; source: DashboardSource; tag: string | null; size?: CountryFlagSize }) {
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
