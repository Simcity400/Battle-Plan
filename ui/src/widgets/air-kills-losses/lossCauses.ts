const NAMES: Readonly<Record<number, string>> = {
  1: 'Aircraft combat',
  2: 'Anti-air / raid effect',
  3: 'Air accidents',
  4: 'Airbase bombing',
  5: 'Train anti-air',
  6: 'Kamikaze anti-air',
};

export const airCauseName = (cause: number): string => NAMES[cause] ?? 'Other';
