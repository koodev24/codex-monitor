import { useId } from "react";
import { Bell, CloudDownload, PanelTop } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";
import { useAppDispatch, useAppSelector } from "@/store/hooks";
import { saveSettings, type SettingsPatch } from "@/store/snapshotSlice";
import { selectBusy, setSettingsOpen } from "@/store/uiSlice";

function SettingRow({
  icon,
  title,
  description,
  checked,
  disabled,
  onChange,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  const descriptionId = useId();
  return (
    <div className="flex items-start gap-3 rounded-xl border bg-muted/20 p-3">
      <span
        className="mt-0.5 grid size-7 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary"
        aria-hidden="true"
      >
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{title}</p>
        <p id={descriptionId} className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={title}
        aria-describedby={descriptionId}
        disabled={disabled}
        onClick={() => onChange(!checked)}
        className={cn(
          "relative mt-0.5 h-5 w-9 shrink-0 rounded-full transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50",
          checked ? "bg-primary" : "bg-muted-foreground/30",
        )}
      >
        <span
          className={cn(
            "absolute top-0.5 size-4 rounded-full bg-background shadow-sm transition-transform",
            checked ? "translate-x-[18px]" : "translate-x-0.5",
          )}
        />
      </button>
    </div>
  );
}

export function SettingsDialog() {
  const dispatch = useAppDispatch();
  const open = useAppSelector((s) => s.ui.settingsOpen);
  const snap = useAppSelector((s) => s.snapshot.snap);
  const busy = useAppSelector(selectBusy("settings"));

  function apply(patch: SettingsPatch) {
    void dispatch(saveSettings(patch));
  }

  return (
    <Dialog open={open} onOpenChange={(next) => dispatch(setSettingsOpen(next))}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Settings</DialogTitle>
          <DialogDescription>
            Preferences are saved automatically and apply immediately.
          </DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          <SettingRow
            icon={<PanelTop className="size-4" />}
            title="Show menu bar item"
            description={
              snap?.show_menu_bar
                ? "Quota percentages stay visible in the menu bar. Closing the window hides it there."
                : "The menu bar item is hidden. Closing the window quits Codex Monitor."
            }
            checked={snap?.show_menu_bar ?? true}
            disabled={busy || !snap}
            onChange={(show_menu_bar) => apply({ show_menu_bar })}
          />
          <SettingRow
            icon={<Bell className="size-4" />}
            title="Desktop notifications"
            description="Notify when Codex refreshes its saved session in the background."
            checked={snap?.notifications_enabled ?? true}
            disabled={busy || !snap}
            onChange={(notifications_enabled) => apply({ notifications_enabled })}
          />
          <SettingRow
            icon={<CloudDownload className="size-4" />}
            title="Automatic update checks"
            description="Check the signed update feed at launch and every 6 hours. Manual checks stay available."
            checked={snap?.auto_check_updates ?? true}
            disabled={busy || !snap}
            onChange={(auto_check_updates) => apply({ auto_check_updates })}
          />
        </div>

        <DialogFooter>
          <Button onClick={() => dispatch(setSettingsOpen(false))}>Done</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
