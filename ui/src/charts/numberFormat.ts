export const usNumber = new Intl.NumberFormat('en-US');
export const usWhole = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });
export const gbNumber = new Intl.NumberFormat('en-GB');
export const gbTwoSignificant = new Intl.NumberFormat('en-GB', { maximumSignificantDigits: 2 });

export function formatMegabytes(bytes: number): string {
  const megabytes = Math.ceil(bytes / 2 ** 20);
  return megabytes < 1024 ? `${megabytes} MB` : `${+(megabytes / 1024).toFixed(1)} GB`;
}
