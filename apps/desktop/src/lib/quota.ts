/**
 * Shared quota presentation rules. Both windows use these so a percentage
 * never means one colour in the dashboard and another in the menu bar.
 */
export function quotaLeft(used?: number | null): number | null {
  if (used === undefined || used === null || !Number.isFinite(used)) return null;
  return Math.round(Math.max(0, Math.min(100, 100 - used)));
}

// Tailwind's semantic quota colours, not theme tokens: green means plenty
// left, amber means watch it, and red means nearly gone. The thresholds sit
// where a quota bar stops being reassuring, not at arbitrary round numbers.
export const QUOTA_TONE = {
  unknown: "[&_[data-slot=progress-indicator]]:bg-muted-foreground/40",
  low: "[&_[data-slot=progress-indicator]]:bg-red-500",
  medium: "[&_[data-slot=progress-indicator]]:bg-amber-500",
  high: "[&_[data-slot=progress-indicator]]:bg-emerald-500",
} as const;

export function quotaTone(left: number | null): string {
  if (left === null) return QUOTA_TONE.unknown;
  if (left <= 20) return QUOTA_TONE.low;
  if (left <= 50) return QUOTA_TONE.medium;
  return QUOTA_TONE.high;
}
