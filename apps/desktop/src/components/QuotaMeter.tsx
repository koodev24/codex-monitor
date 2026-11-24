import { Progress } from "@/components/ui/progress";
import { useI18n } from "@/i18n";
import { formatResetCountdown, formatResetStamp } from "@/lib/format";
import { quotaLeft, quotaTone } from "@/lib/quota";
import { cn } from "@/lib/utils";

/**
 * One quota window: `5h·bar·39%·Reset in 1h 59m·4:36 PM`.
 * Compact mode is a flex row for the menu bar. The dashboard uses a fixed
 * five-column grid so labels, bars, percentages, and reset details align
 * across rows without text length changing the bar width.
 */
export function QuotaMeter({
  label,
  used,
  resetTs,
  now,
  compact = false,
  showStamp = true,
  className,
}: {
  label: string;
  used?: number | null;
  resetTs?: number;
  now: number;
  compact?: boolean;
  showStamp?: boolean;
  className?: string;
}) {
  const { locale, t } = useI18n();
  const left = quotaLeft(used);
  const gridSeparator =
    "relative before:absolute before:top-1/2 before:-left-1 before:-translate-x-1/2 before:-translate-y-1/2 before:text-muted-foreground/40 before:content-['·']";
  return (
    <span
      className={cn(
        "w-full min-w-0 items-center tabular-nums text-muted-foreground",
        compact
          ? "flex gap-0.5 text-[9px] leading-none"
          : "grid grid-cols-[1.25rem_minmax(4rem,1fr)_2.5rem_7.25rem_4.75rem] gap-x-1.5 text-[11px]",
        className,
      )}
    >
      <span
        className={cn(
          "shrink-0 font-medium text-foreground/70",
          compact && "w-3",
        )}
      >
        {label}
      </span>
      {compact && (
        <span aria-hidden className="shrink-0 opacity-40">
          ·
        </span>
      )}
      <Progress
        value={left}
        aria-label={t("quota.ariaRemaining", { label })}
        className={cn(
          "shrink-0 gap-0",
          compact
            ? "w-12 shrink-0"
            : cn("h-1.5 w-full [&_[data-slot=progress-track]]:h-1.5", gridSeparator),
          quotaTone(left),
        )}
      />
      {compact && (
        <span aria-hidden className="shrink-0 opacity-40">
          ·
        </span>
      )}
      <span
        className={cn(
          "shrink-0 font-medium text-foreground",
          compact ? "" : cn("text-right", gridSeparator),
        )}
      >
        {left === null ? "—" : `${left}%`}
      </span>
      {compact && (
        <span aria-hidden className="shrink-0 opacity-40">
          ·
        </span>
      )}
      <span
        className={cn(
          "min-w-0 truncate whitespace-nowrap",
          !compact && gridSeparator,
        )}
      >
        {resetTs
          ? t("quota.resetIn", { time: formatResetCountdown(resetTs, now, locale) })
          : t("quota.resetUnknown")}
      </span>
      {showStamp && resetTs ? (
        <>
          {compact && (
            <span aria-hidden className="shrink-0 opacity-40">
              ·
            </span>
          )}
          <span className={cn("min-w-0 truncate", !compact && gridSeparator)}>
            {formatResetStamp(resetTs, now, locale)}
          </span>
        </>
      ) : null}
    </span>
  );
}
