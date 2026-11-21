export default function BrandMark({ size = 30 }: { size?: number }) {
  return <img className="brand-mark" src="/favicon.svg" width={size} height={size} alt="" aria-hidden="true" draggable={false} />;
}
