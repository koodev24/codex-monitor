import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { sendNotification } from "@tauri-apps/plugin-notification";
import { openUrl } from "@tauri-apps/plugin-opener";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import {
  Archive,
  ArchiveRestore,
  ArrowDownToLine,
  ArrowLeftRight,
  Check,
  Coins,
  Copy,
  Download,
  Eraser,
  Eye,
  EyeOff,
  Moon,
  RefreshCw,
  ScrollText,
  Sun,
  Trash2,
  Upload,
  UserPlus,
} from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from "@/components/ui/resizable";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  formatCreditExpires,
  formatGrantedAt,
  formatQuotaLeft,
  formatResetDisplay,
  formatTimeRemaining,
  resetTsOf,
  soonestExpiringCredit,
  usedOf,
  weeklyOf,
  type AccountUsage,
  type ResetCreditsPayload,
} from "./lib/format";
import { api, type AuthOutcome, type Snapshot } from "./lib/tauri";

type SortKey = "email" | "quota" | "reset";
type ConfirmAction =
  | { kind: "remove"; email: string }
  | { kind: "archive"; email: string; archived: boolean }
  | { kind: "switch"; email: string }
  | null;

const MISSING_TOKEN_RETRIES = 6;

function useTheme() {
  const [dark, setDark] = useState(() => {
    const saved = localStorage.getItem("cm-theme");
    if (saved) return saved === "dark";
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  });
  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
    localStorage.setItem("cm-theme", dark ? "dark" : "light");
  }, [dark]);
  return { dark, toggle: () => setDark((d) => !d) };
}

function IconBtn({
  title,
  onClick,
  disabled,
  variant = "ghost",
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  variant?: "ghost" | "outline" | "default" | "destructive" | "secondary";
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button variant={variant} size="icon" onClick={onClick} disabled={disabled} aria-label={title}>
            {children}
          </Button>
        }
      />
      <TooltipContent>{title}</TooltipContent>
    </Tooltip>
  );
}

export default function App() {
  const [snap, setSnap] = useState<Snapshot | null>(null);
  const [status, setStatus] = useState("Starting…");
  const [busy, setBusy] = useState(false);
  const [sortKey, setSortKey] = useState<SortKey | null>(null);
  const [sortAsc, setSortAsc] = useState(true);
  const [confirm, setConfirm] = useState<ConfirmAction>(null);
  const [resetsEmail, setResetsEmail] = useState<string | null>(null);
  const [resets, setResets] = useState<ResetCreditsPayload | null>(null);
  const [logsOpen, setLogsOpen] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [loginOpen, setLoginOpen] = useState(false);
  const [loginLines, setLoginLines] = useState<string[]>([]);
  const [loginDone, setLoginDone] = useState<string | null>(null);
  const [updateReady, setUpdateReady] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);
  const [switchInfo, setSwitchInfo] = useState<string | null>(null);
  const { dark, toggle } = useTheme();
  const missingRetries = useRef(0);
  const urlOpened = useRef(false);
  const sortTimer = useRef<number | undefined>(undefined);

  const refresh = useCallback(async () => {
    try {
      const s = await api.snapshot();
      setSnap(s);
      setSortKey(
        s.sort_column === "weekly_quota" || s.sort_column === "quota"
          ? "quota"
          : s.sort_column === "weekly_reset"
            ? "reset"
            : s.sort_column === "email"
              ? "email"
              : null,
      );
      setSortAsc(s.sort_asc);
      setLogsOpen(s.logs_expanded);
    } catch (e) {
      setStatus(`Failed to load state: ${e}`);
    }
  }, []);

  const handleOutcome = useCallback(
    async (o: AuthOutcome) => {
      if (o.kind === "NoChange") return;
      if (o.kind === "MissingToken") {
        if (missingRetries.current < MISSING_TOKEN_RETRIES) {
          missingRetries.current += 1;
          setStatus("Auth file changing, retrying read…");
          setTimeout(async () => {
            try {
              await handleOutcome(await api.processAuthFile());
            } catch {
              /* next poll */
            }
          }, 350);
        } else {
          missingRetries.current = 0;
          await refresh();
        }
        return;
      }
      missingRetries.current = 0;
      if (o.kind === "AuthRefreshed") {
        setStatus(o.message);
        void sendNotification({ title: "Codex Account Monitor", body: o.message });
      } else if (o.kind === "Fetched" || o.kind === "LoggedOut") {
        setStatus(o.message);
      } else if (o.kind === "ParseError" || o.kind === "NoFile") {
        setStatus(o.kind === "NoFile" ? "Auth file removed. Signed out." : o.message);
      }
      await refresh();
    },
    [refresh],
  );

  const poll = useCallback(async () => {
    try {
      await handleOutcome(await api.processAuthFile());
    } catch {
      /* transient */
    }
    try {
      const r = await api.checkAutoFetch();
      if (r) {
        setStatus(r.message);
        await refresh();
      }
    } catch (e) {
      setStatus(`Auto-fetch failed: ${e}`);
    }
  }, [handleOutcome, refresh]);

  useEffect(() => {
    refresh().then(() => poll());
    const id = window.setInterval(poll, 5000);
    const unlisteners: Array<() => void> = [];
    listen("auth-file-changed", () => void poll()).then((u) => unlisteners.push(u));
    listen<string>("codex-login-output", (e) =>
      setLoginLines((lines) => [...lines.slice(-200), e.payload]),
    ).then((u) => unlisteners.push(u));
    listen<string>("codex-login-url", (e) => {
      if (!urlOpened.current) {
        urlOpened.current = true;
        void openUrl(e.payload).catch(() => undefined);
      }
    }).then((u) => unlisteners.push(u));
    listen<{ ok: boolean; message: string }>("codex-login-done", (e) => {
      setLoginDone(e.payload.message);
      setStatus(e.payload.message);
      void refresh();
    }).then((u) => unlisteners.push(u));
    return () => {
      window.clearInterval(id);
      unlisteners.forEach((u) => u());
    };
  }, [poll, refresh]);

  const checkUpdates = useCallback(async (manual: boolean) => {
    if (manual) setCheckingUpdate(true);
    try {
      const update = await check();
      if (update) {
        setUpdateReady(`Update ${update.version} ready to install.`);
        setStatus(`Update ${update.version} is available.`);
        if (manual) {
          await update.downloadAndInstall((ev) => {
            if (ev.event === "Finished") setStatus("Update installed, restarting…");
          });
          await relaunch();
        }
      } else {
        setUpdateReady(null);
        if (manual) setStatus("You're already on the latest version.");
      }
    } catch (e) {
      if (manual) setStatus(`Update check failed: ${e}`);
    } finally {
      if (manual) setCheckingUpdate(false);
    }
  }, []);

  useEffect(() => {
    const t = window.setTimeout(() => void checkUpdates(false), 1500);
    const id = window.setInterval(() => void checkUpdates(false), 6 * 3600 * 1000);
    return () => {
      window.clearTimeout(t);
      window.clearInterval(id);
    };
  }, [checkUpdates]);

  const installUpdate = useCallback(async () => {
    setCheckingUpdate(true);
    try {
      const update = await check();
      if (!update) {
        setStatus("You're already on the latest version.");
        return;
      }
      await update.downloadAndInstall();
      await relaunch();
    } catch (e) {
      setStatus(`Update failed: ${e}`);
    } finally {
      setCheckingUpdate(false);
    }
  }, []);

  const rows = useMemo(() => {
    if (!snap) return [];
    const entries = Object.entries(snap.accounts);
    const visible = snap.show_archived ? entries : entries.filter(([, a]) => !a.archived);
    const keyOf = (email: string, a: AccountUsage): string | number => {
      if (sortKey === "email") return email.toLowerCase();
      if (sortKey === "quota") return usedOf(a) ?? -1;
      return resetTsOf(a);
    };
    return visible.sort(([ea, aa], [eb, ab]) => {
      if (ea === snap.current_email) return -1;
      if (eb === snap.current_email) return 1;
      if ((aa.archived ?? false) !== (ab.archived ?? false)) {
        return (aa.archived ?? false) ? 1 : -1;
      }
      if (!sortKey) return resetTsOf(aa) - resetTsOf(ab);
      const ka = keyOf(ea, aa);
      const kb = keyOf(eb, ab);
      const cmp = ka < kb ? -1 : ka > kb ? 1 : 0;
      return sortAsc ? cmp : -cmp;
    });
  }, [snap, sortKey, sortAsc]);

  const onHeader = (key: SortKey) => {
    let nextKey: SortKey | null = key;
    let nextAsc = true;
    if (sortKey === key) {
      if (sortAsc) nextAsc = false;
      else nextKey = null;
    }
    setSortKey(nextKey);
    setSortAsc(nextAsc);
    window.clearTimeout(sortTimer.current);
    sortTimer.current = window.setTimeout(() => {
      const col = nextKey === "quota" ? "weekly_quota" : nextKey === "reset" ? "weekly_reset" : nextKey;
      void api.saveSort(col, nextAsc).catch(() => undefined);
    }, 500);
  };

  const arrow = (key: SortKey) => (sortKey === key ? (sortAsc ? " ▲" : " ▼") : " ↕");

  async function doFetch(email?: string) {
    setBusy(true);
    try {
      const r = email ? await api.fetchBackup(email) : await api.manualFetch();
      setStatus(r.message);
    } catch (e) {
      setStatus(`Fetch failed: ${e}`);
    } finally {
      setBusy(false);
      await refresh();
    }
  }

  async function doConfirm() {
    if (!confirm) return;
    setBusy(true);
    try {
      if (confirm.kind === "remove") {
        setStatus(await api.removeAccount(confirm.email));
      } else if (confirm.kind === "archive") {
        setStatus(await api.setArchived(confirm.email, !confirm.archived));
      } else {
        const msg = await api.switchAccount(confirm.email);
        setStatus(msg);
        setSwitchInfo(confirm.email);
      }
    } catch (e) {
      setStatus(`Action failed: ${e}`);
    } finally {
      setConfirm(null);
      setBusy(false);
      await refresh();
    }
  }

  async function openResets(email: string) {
    try {
      setResets(await api.resets(email));
      setResetsEmail(email);
    } catch (e) {
      setStatus(`Could not load reset credits: ${e}`);
    }
  }

  async function doExport() {
    const path = await save({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path) return;
    try {
      setStatus(await api.exportData(path));
    } catch (e) {
      setStatus(`Export failed: ${e}`);
    }
  }

  async function doImport() {
    const path = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path || Array.isArray(path)) return;
    try {
      setStatus(await api.importData(path));
      await refresh();
    } catch (e) {
      setStatus(`Import failed: ${e}`);
    }
  }

  async function toggleLogs() {
    const next = !logsOpen;
    setLogsOpen(next);
    await api.saveLogsExpanded(next).catch(() => undefined);
    if (next) {
      try {
        setLogs(await api.logs());
      } catch (e) {
        setStatus(`Could not load logs: ${e}`);
      }
    }
  }

  function startLogin() {
    setLoginLines([]);
    setLoginDone(null);
    urlOpened.current = false;
    setLoginOpen(true);
    api.loginStart().catch((e) => setLoginLines([`Failed to start login: ${e}`]));
  }

  const now = Date.now() / 1000;
  const soonest = resetsEmail && snap ? soonestExpiringCredit(snap.accounts[resetsEmail]?.resets) : undefined;

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col gap-2 bg-background p-2 text-foreground">
        <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
          <ResizablePanel defaultSize={62} minSize={25}>
            <div className="flex h-full flex-col overflow-hidden rounded-xl border bg-card">
              <div className="min-h-0 flex-1 overflow-y-auto">
                {rows.length === 0 ? (
                  <div className="flex h-48 flex-col items-center justify-center gap-3 text-muted-foreground">
                    <p className="text-sm">
                      {snap && !snap.auth_file_exists
                        ? "Not signed in — the Codex auth file is missing."
                        : "No accounts yet. Fetch quota or add an account."}
                    </p>
                    <Button onClick={startLogin}>
                      <UserPlus />
                      Add account
                    </Button>
                  </div>
                ) : (
                  <Table>
                    <TableHeader className="sticky top-0 bg-muted">
                      <TableRow>
                        <TableHead>
                          <button className="font-bold hover:text-primary" onClick={() => onHeader("email")}>
                            Account Email{arrow("email")}
                          </button>
                        </TableHead>
                        <TableHead>
                          <button className="font-bold hover:text-primary" onClick={() => onHeader("quota")}>
                            Quota{arrow("quota")}
                          </button>
                        </TableHead>
                        <TableHead>
                          <button className="font-bold hover:text-primary" onClick={() => onHeader("reset")}>
                            Reset{arrow("reset")}
                          </button>
                        </TableHead>
                        <TableHead className="text-right">Action</TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {rows.map(([email, a]) => {
                        const isCurrent = email === snap?.current_email;
                        const weekly = weeklyOf(a);
                        return (
                          <TableRow key={email} className={isCurrent ? "bg-emerald-500/10" : undefined}>
                            <TableCell>
                              <span className="flex min-w-0 items-center gap-2">
                                <span className="truncate font-medium" title={email}>
                                  {email}
                                </span>
                                {isCurrent && <Badge variant="outline">current</Badge>}
                                {a.archived && <Badge variant="secondary">arch</Badge>}
                              </span>
                            </TableCell>
                            <TableCell title={weekly ? `Used ${weekly.used_percent ?? "?"}%` : "Fetch quota first"}>
                              {formatQuotaLeft(usedOf(a))}
                            </TableCell>
                            <TableCell className="max-w-56 truncate" title={formatResetDisplay(resetTsOf(a), now)}>
                              {formatResetDisplay(resetTsOf(a), now)}
                            </TableCell>
                            <TableCell>
                              <span className="flex justify-end gap-0.5">
                                <IconBtn
                                  title={isCurrent ? "Fetch quota" : "Fetch this backup account"}
                                  onClick={() => void doFetch(isCurrent ? undefined : email)}
                                  disabled={busy}
                                >
                                  <RefreshCw />
                                </IconBtn>
                                <IconBtn title="Reset credits" onClick={() => void openResets(email)}>
                                  <Coins />
                                </IconBtn>
                                {!isCurrent && (
                                  <IconBtn
                                    title="Switch to this account"
                                    onClick={() => setConfirm({ kind: "switch", email })}
                                    disabled={busy}
                                  >
                                    <ArrowLeftRight />
                                  </IconBtn>
                                )}
                                <IconBtn
                                  title={a.archived ? "Unarchive account" : "Archive account"}
                                  onClick={() => setConfirm({ kind: "archive", email, archived: !!a.archived })}
                                >
                                  {a.archived ? <ArchiveRestore /> : <Archive />}
                                </IconBtn>
                                <IconBtn title="Remove account" onClick={() => setConfirm({ kind: "remove", email })}>
                                  <Trash2 />
                                </IconBtn>
                              </span>
                            </TableCell>
                          </TableRow>
                        );
                      })}
                    </TableBody>
                  </Table>
                )}
              </div>
            </div>
          </ResizablePanel>

          <ResizableHandle withHandle />

          <ResizablePanel defaultSize={38} minSize={18}>
            <div className="flex h-full min-h-0 flex-col gap-2 overflow-hidden rounded-xl border bg-card p-2">
              <p className="truncate px-1 text-xs text-muted-foreground" title={status}>
                {status}
              </p>
              <div className="flex flex-wrap items-center gap-1">
                <IconBtn title="Copy status" onClick={() => void navigator.clipboard.writeText(status)}>
                  <Copy />
                </IconBtn>
                <IconBtn title={logsOpen ? "Hide logs" : "Show logs"} onClick={() => void toggleLogs()}>
                  <ScrollText />
                </IconBtn>
                {logsOpen && (
                  <IconBtn
                    title="Clear logs"
                    onClick={() => {
                      void api.clearLogs().then(() => setLogs([]));
                    }}
                  >
                    <Eraser />
                  </IconBtn>
                )}
                <IconBtn title="Export data" onClick={() => void doExport()}>
                  <Download />
                </IconBtn>
                <IconBtn title="Import data" onClick={() => void doImport()}>
                  <Upload />
                </IconBtn>
                <IconBtn title="Add account via Codex login" onClick={startLogin}>
                  <UserPlus />
                </IconBtn>
                <span className="flex items-center gap-1.5 pl-1">
                  <Label htmlFor="auto-fetch" className="text-xs text-muted-foreground">
                    Auto
                  </Label>
                  <Select
                    value={snap?.auto_fetch ?? "None"}
                    onValueChange={(v) => {
                      const val = v ?? "None";
                      void api
                        .saveAutoFetch(val)
                        .then((m) => {
                          setStatus(m);
                          void refresh();
                        })
                        .catch((err) => setStatus(`Auto-fetch failed: ${err}`));
                    }}
                  >
                    <SelectTrigger id="auto-fetch" className="h-8 w-24" title="Auto-fetch interval for the active account">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(snap?.auto_fetch_options ?? ["None"]).map((o) => (
                        <SelectItem key={o} value={o}>
                          {o}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </span>
                <IconBtn title="Fetch quota now" onClick={() => void doFetch()} disabled={busy}>
                  <RefreshCw />
                </IconBtn>
                <IconBtn
                  title={snap?.show_archived ? "Hide archived accounts" : "Show archived accounts"}
                  onClick={() => {
                    const next = !(snap?.show_archived ?? false);
                    void api.saveShowArchived(next).then(() => void refresh());
                  }}
                >
                  {snap?.show_archived ? <EyeOff /> : <Eye />}
                </IconBtn>
                <IconBtn
                  title="Check for updates"
                  onClick={() => void checkUpdates(true)}
                  disabled={checkingUpdate}
                >
                  <Check />
                </IconBtn>
                {updateReady && (
                  <IconBtn title={updateReady} onClick={() => void installUpdate()}>
                    <ArrowDownToLine />
                  </IconBtn>
                )}
                <IconBtn title="Toggle theme" onClick={toggle}>
                  {dark ? <Sun /> : <Moon />}
                </IconBtn>
              </div>
              {logsOpen && (
                <ScrollArea className="min-h-40 flex-1 rounded-md border bg-muted/30">
                  <pre className="whitespace-pre-wrap p-2 text-xs text-muted-foreground">
                    {logs.length === 0 ? "(no log entries yet)" : logs.join("\n")}
                  </pre>
                </ScrollArea>
              )}
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>

        <Dialog open={confirm !== null} onOpenChange={(o) => !o && setConfirm(null)}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Confirm</DialogTitle>
              <DialogDescription>
                {confirm?.kind === "remove" && `Remove ${confirm.email}? Quota history is deleted.`}
                {confirm?.kind === "archive" &&
                  `${confirm.archived ? "Unarchive" : "Archive"} ${confirm.email}?`}
                {confirm?.kind === "switch" && `Switch the active Codex account to ${confirm.email}?`}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setConfirm(null)}>
                Cancel
              </Button>
              <Button onClick={() => void doConfirm()} disabled={busy}>
                Confirm
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog open={resetsEmail !== null} onOpenChange={(o) => !o && (setResetsEmail(null), setResets(null))}>
          <DialogContent className="max-w-xl">
            <DialogHeader>
              <DialogTitle>Reset Credits — {resetsEmail}</DialogTitle>
            </DialogHeader>
            {!resets?.credits?.length ? (
              <p className="text-sm text-muted-foreground">No reset credits found. Fetch quota first.</p>
            ) : (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Expires</TableHead>
                    <TableHead>Remaining</TableHead>
                    <TableHead>Granted</TableHead>
                    <TableHead>Status</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {resets.credits.map((c, i) => (
                    <TableRow key={i}>
                      <TableCell>{formatCreditExpires(c.expires_at, now)}</TableCell>
                      <TableCell>{formatTimeRemaining(c.expires_at, now)}</TableCell>
                      <TableCell>{formatGrantedAt(c.granted_at)}</TableCell>
                      <TableCell>{c.status ?? "unknown"}</TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
            {soonest && (
              <p className="text-xs text-muted-foreground" title="Soonest expiring available credit">
                Soonest expiry in {formatTimeRemaining(soonest.expires_at, now)}.
              </p>
            )}
          </DialogContent>
        </Dialog>

        <Dialog
          open={loginOpen}
          onOpenChange={(o) => {
            if (!o) {
              if (!loginDone) void api.loginCancel();
              setLoginOpen(false);
            }
          }}
        >
          <DialogContent className="max-w-xl">
            <DialogHeader>
              <DialogTitle>Add account — Codex login</DialogTitle>
            </DialogHeader>
            <ScrollArea className="h-56 rounded-md border bg-slate-950">
              <pre className="whitespace-pre-wrap p-2 text-xs text-slate-200">
                {loginLines.length === 0 ? "Starting login…" : loginLines.join("\n")}
              </pre>
            </ScrollArea>
            <DialogFooter>
              {loginDone ? (
                <Button onClick={() => setLoginOpen(false)}>Close</Button>
              ) : (
                <Button variant="outline" onClick={() => void api.loginCancel()}>
                  Cancel
                </Button>
              )}
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog open={switchInfo !== null} onOpenChange={(o) => !o && setSwitchInfo(null)}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Switched account</DialogTitle>
              <DialogDescription>
                Now using {switchInfo}. Restart the Codex app so it picks up the new auth?
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => setSwitchInfo(null)}>
                Later
              </Button>
              <Button
                onClick={() => {
                  void api
                    .restartCodex()
                    .then((m) => setStatus(m))
                    .catch((e) => setStatus(`Restart failed: ${e}`));
                  setSwitchInfo(null);
                }}
              >
                Restart Codex
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </TooltipProvider>
  );
}
