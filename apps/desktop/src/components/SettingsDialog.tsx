import { useId } from "react";
import { Bell, CloudDownload, Languages, PanelTop } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { LOCALE_LABELS, LOCALES, useI18n } from "@/i18n";
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

function LanguageRow({
  value,
  disabled,
  onChange,
}: {
  value: string;
  disabled: boolean;
  onChange: (locale: string) => void;
}) {
  const { t } = useI18n();
  const descriptionId = useId();
  const items = {
    system: t("settings.systemLanguage"),
    ...LOCALE_LABELS,
  };
  return (
    <div className="flex items-start gap-3 rounded-xl border bg-muted/20 p-3">
      <span
        className="mt-0.5 grid size-7 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary"
        aria-hidden="true"
      >
        <Languages className="size-4" />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{t("settings.language")}</p>
        <p id={descriptionId} className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
          {t("settings.languageDescription")}
        </p>
      </div>
      <Select
        items={items}
        value={value}
        onValueChange={(next) => {
          if (next) onChange(next);
        }}
        disabled={disabled}
      >
        <SelectTrigger
          className="mt-0.5 h-8 w-32 shrink-0 text-xs"
          aria-label={t("settings.language")}
          aria-describedby={descriptionId}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="system">{items.system}</SelectItem>
          {LOCALES.map((locale) => (
            <SelectItem key={locale} value={locale}>
              {LOCALE_LABELS[locale]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

export function SettingsDialog() {
  const dispatch = useAppDispatch();
  const open = useAppSelector((s) => s.ui.settingsOpen);
  const snap = useAppSelector((s) => s.snapshot.snap);
  const busy = useAppSelector(selectBusy("settings"));
  const { t } = useI18n();

  function apply(patch: SettingsPatch) {
    void dispatch(saveSettings(patch));
  }

  return (
    <Dialog open={open} onOpenChange={(next) => dispatch(setSettingsOpen(next))}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("settings.title")}</DialogTitle>
          <DialogDescription>{t("settings.description")}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2">
          <LanguageRow
            value={snap?.locale ?? "system"}
            disabled={busy || !snap}
            onChange={(locale) => apply({ locale })}
          />
          <SettingRow
            icon={<PanelTop className="size-4" />}
            title={t("settings.menuBar")}
            description={
              snap?.show_menu_bar ? t("settings.menuBarOn") : t("settings.menuBarOff")
            }
            checked={snap?.show_menu_bar ?? true}
            disabled={busy || !snap}
            onChange={(show_menu_bar) => apply({ show_menu_bar })}
          />
          <SettingRow
            icon={<Bell className="size-4" />}
            title={t("settings.notifications")}
            description={t("settings.notificationsDescription")}
            checked={snap?.notifications_enabled ?? true}
            disabled={busy || !snap}
            onChange={(notifications_enabled) => apply({ notifications_enabled })}
          />
          <SettingRow
            icon={<CloudDownload className="size-4" />}
            title={t("settings.autoUpdates")}
            description={t("settings.autoUpdatesDescription")}
            checked={snap?.auto_check_updates ?? true}
            disabled={busy || !snap}
            onChange={(auto_check_updates) => apply({ auto_check_updates })}
          />
        </div>

        <DialogFooter>
          <Button onClick={() => dispatch(setSettingsOpen(false))}>{t("action.done")}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
