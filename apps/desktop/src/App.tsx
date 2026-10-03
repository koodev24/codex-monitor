import { useEffect, useMemo, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
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
  Loader2,
  LogOut,
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
} from "./lib/format";
import { api } from "./lib/tauri";
import { useAppDispatch, useAppSelector } from "./store/hooks";
import {
  appendLines,
  cancelLogin,
  copyLoginUrl,
  markUrlOpened,
  openLoginUrlOrDefault,
  resetLines,
  setDone,
  setOpen as setLoginOpen,
  setUrl,
  startLogin,
} from "./store/loginSlice";
import { clearLogs, refreshLogs, toggleLogs } from "./store/logsSlice";
import {
  cycleSort,
  doExport,
  doImport,
  fetchAllAccounts,
  loadSnapshot,
  logout,
  manualFetch,
  openResets,
  pollOnce,
  runConfirmAction,
  saveAutoFetchValue,
  setLoadError,
  toggleArchivedVisibility,
  type SortKey,
} from "./store/snapshotSlice";
import {
  checkUpdates,
  installUpdate,
  selectAnyBusy,
  selectBusy,
  setConfirm,
  setResetsView,
  setStatus,
  setSwitchInfo,
} from "./store/uiSlice";

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

function Spin({ className }: { className?: string }) {
  return <Loader2 aria-hidden className={`cm-spin ${className ?? ""}`} />;
}

function IconBtn({
  title,
  onClick,
  busyKey,
  variant = "ghost",
  children,
}: {
  title: string;
  onClick: () => void;
  busyKey?: string;
  variant?: "ghost" | "outline" | "default" | "destructive" | "secondary";
  children: React.ReactNode;
}) {
  const busy = useAppSelector((s) => (busyKey ? !!s.ui.busy[busyKey] : false));
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant={variant}
            size="icon"
            onClick={onClick}
            disabled={busy}
            aria-label={busy ? `${title} (working…)` : title}
          >
            {busy ? <Spin /> : children}
          </Button>
        }
      />
      <TooltipContent>{busy ? `${title} (working…)` : title}</TooltipContent>
    </Tooltip>
  );
}

export default function App() {
  const dispatch = useAppDispatch();
  const snap = useAppSelector((s) => s.snapshot.snap);
  const initialized = useAppSelector((s) => s.snapshot.initialized);
  const loadError = useAppSelector((s) => s.snapshot.loadError);
  const sortKey = useAppSelector((s) => s.snapshot.sortKey);
  const sortAsc = useAppSelector((s) => s.snapshot.sortAsc);
  const status = useAppSelector((s) => s.ui.status);
  const anyBusy = useAppSelector(selectAnyBusy);
  const confirm = useAppSelector((s) => s.ui.confirm);
  const resetsEmail = useAppSelector((s) => s.ui.resetsEmail);
  const resets = useAppSelector((s) => s.ui.resets);
  const switchInfo = useAppSelector((s) => s.ui.switchInfo);
  const updateReady = useAppSelector((s) => s.ui.updateReady);
  const updateProgress = useAppSelector((s) => s.ui.updateProgress);
  const logsOpen = useAppSelector((s) => s.logs.open);
  const logs = useAppSelector((s) => s.logs.entries);
  const loginOpen = useAppSelector((s) => s.login.open);
  const loginLines = useAppSelector((s) => s.login.lines);
  const loginUrl = useAppSelector((s) => s.login.url);
  const fetchFailed = useAppSelector((s) => s.snapshot.fetchFailed);
  const loginDone = useAppSelector((s) => s.login.done);
  const loginStarting = useAppSelector((s) => s.login.starting);
  const confirmBusy = useAppSelector(selectBusy("confirm"));
  const { dark, toggle } = useTheme();

  useEffect(() => {
    if (!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
      dispatch(
        setLoadError(
          "Not running inside Tauri (no backend bridge). Run `npm run tauri dev`, not `npm run dev`.",
        ),
      );
      dispatch(setStatus("Not running inside Tauri."));
      return;
    }
    void dispatch(loadSnapshot()).then(() => void dispatch(pollOnce()));
    const id = window.setInterval(() => {
      void dispatch(pollOnce());
      void dispatch(refreshLogs());
    }, 5000);
    // StrictMode mounts, unmounts, and remounts this effect while the
    // listen() promises are still pending; without the cancelled flag the
    // first set of listeners never unsubscribes and every event fires twice.
    let cancelled = false;
    const unlisteners: Array<() => void> = [];
    const track = (p: Promise<() => void>) => {
      void p.then((u) => {
        if (cancelled) u();
        else unlisteners.push(u);
      });
    };
    track(
      listen("auth-file-changed", () => {
        void dispatch(pollOnce());
        void dispatch(refreshLogs());
      }),
    );
    track(
      listen<string>("codex-login-output", (e) => {
        dispatch(appendLines([e.payload]));
      }),
    );
    track(
      listen<string>("codex-login-url", (e) => {
        dispatch((_, getState) => {
          if (!getState().login.urlOpened) {
            dispatch(markUrlOpened());
            dispatch(setUrl(e.payload));
            dispatch(setStatus("Login page ready — copy the URL or open it below."));
          }
        });
      }),
    );
    track(
      listen<{ ok: boolean; message: string }>("codex-login-done", (e) => {
        dispatch(setDone(e.payload.message));
        dispatch(setStatus(e.payload.message));
        if (e.payload.ok) dispatch(setLoginOpen(false));
        void dispatch(loadSnapshot());
        void dispatch(refreshLogs());
      }),
    );
    return () => {
      cancelled = true;
      window.clearInterval(id);
      unlisteners.forEach((u) => u());
    };
  }, [dispatch]);

  useEffect(() => {
    const t = window.setTimeout(() => void dispatch(checkUpdates(false)), 1500);
    const id = window.setInterval(() => void dispatch(checkUpdates(false)), 6 * 3600 * 1000);
    return () => {
      window.clearTimeout(t);
      window.clearInterval(id);
    };
  }, [dispatch]);

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

  const arrow = (key: SortKey) => (sortKey === key ? (sortAsc ? " ▲" : " ▼") : " ↕");

  async function pickExportFile() {
    const path = await save({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (path) void dispatch(doExport(path));
  }

  async function pickImportFile() {
    const path = await open({ filters: [{ name: "JSON", extensions: ["json"] }] });
    if (!path || Array.isArray(path)) return;
    void dispatch(doImport(path));
  }

  function startLoginUi() {
    dispatch(resetLines());
    void dispatch(startLogin());
  }

  const now = Date.now() / 1000;
  const soonest =
    resetsEmail && snap ? soonestExpiringCredit(snap.accounts[resetsEmail]?.resets) : undefined;

  return (
    <TooltipProvider>
      <div className="flex h-screen flex-col gap-2 bg-background p-2 text-foreground">
        <ResizablePanelGroup orientation="vertical" className="min-h-0 flex-1">
          <ResizablePanel defaultSize={62} minSize={25}>
            <div className="flex h-full flex-col overflow-hidden rounded-xl border bg-card">
              <div className="min-h-0 flex-1 overflow-auto">
                {!initialized ? (
                  loadError ? (
                    <div className="flex h-48 flex-col items-center justify-center gap-3 px-6 text-center">
                      <p className="text-sm font-bold">Could not start the app</p>
                      <p className="max-w-md text-xs text-muted-foreground" title={loadError}>
                        {loadError}
                      </p>
                      <Button
                        onClick={() => {
                          dispatch(setLoadError(null));
                          void dispatch(loadSnapshot()).then(() => void dispatch(pollOnce()));
                        }}
                      >
                        <RefreshCw />
                        Retry
                      </Button>
                    </div>
                  ) : (
                    <div className="flex h-48 items-center justify-center gap-2 text-sm text-muted-foreground">
                      <Spin /> Loading accounts…
                    </div>
                  )
                ) : rows.length === 0 ? (
                  <div className="flex h-48 flex-col items-center justify-center gap-3 text-muted-foreground">
                    <p className="text-sm">
                      {snap && !snap.auth_file_exists
                        ? "Not signed in — the Codex auth file is missing."
                        : "No accounts yet. Fetch quota or add an account."}
                    </p>
                    <Button onClick={startLoginUi} disabled={loginStarting}>
                      {loginStarting ? <Spin /> : <UserPlus />}
                      Add account
                    </Button>
                  </div>
                ) : (
                  <Table>
                    <TableHeader className="sticky top-0 bg-muted">
                      <TableRow>
                        <TableHead>
                          <button
                            className="font-bold hover:text-primary"
                            onClick={() => void dispatch(cycleSort("email"))}
                          >
                            Account Email{arrow("email")}
                          </button>
                        </TableHead>
                        <TableHead>
                          <button
                            className="font-bold hover:text-primary"
                            onClick={() => void dispatch(cycleSort("quota"))}
                          >
                            Quota{arrow("quota")}
                          </button>
                        </TableHead>
                        <TableHead>
                          <button
                            className="font-bold hover:text-primary"
                            onClick={() => void dispatch(cycleSort("reset"))}
                          >
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
                          <TableRow
                            key={email}
                            className={isCurrent ? "bg-emerald-500/10" : undefined}
                          >
                            <TableCell>
                              <span className="flex min-w-0 items-center gap-2">
                                <span className="truncate font-medium" title={email}>
                                  {email}
                                </span>
                                {isCurrent && <Badge variant="outline">current</Badge>}
                                {a.archived && <Badge variant="secondary">arch</Badge>}
                              </span>
                            </TableCell>
                            <TableCell
                              title={weekly ? `Used ${weekly.used_percent ?? "?"}%` : "Fetch quota first"}
                            >
                              {formatQuotaLeft(usedOf(a))}
                            </TableCell>
                            <TableCell
                              className="max-w-56 truncate"
                              title={formatResetDisplay(resetTsOf(a), now)}
                            >
                              {formatResetDisplay(resetTsOf(a), now)}
                            </TableCell>
                            <TableCell>
                              <span className="flex justify-end gap-0.5">
                                {(isCurrent || !fetchFailed[email]) && (
                                  <IconBtn
                                    title={isCurrent ? "Fetch quota" : "Fetch this backup account"}
                                    busyKey={isCurrent ? "fetch:all" : `fetch:${email}`}
                                    onClick={() =>
                                      void dispatch(manualFetch(isCurrent ? undefined : email))
                                    }
                                  >
                                    <RefreshCw />
                                  </IconBtn>
                                )}
                                <IconBtn
                                  title="Reset credits"
                                  busyKey={`resets:${email}`}
                                  onClick={() => void dispatch(openResets(email))}
                                >
                                  <Coins />
                                </IconBtn>
                                {!isCurrent && (
                                  <IconBtn
                                    title="Switch to this account"
                                    onClick={() => dispatch(setConfirm({ kind: "switch", email }))}
                                  >
                                    <ArrowLeftRight />
                                  </IconBtn>
                                )}
                                <IconBtn
                                  title={a.archived ? "Unarchive account" : "Archive account"}
                                  onClick={() =>
                                    dispatch(
                                      setConfirm({ kind: "archive", email, archived: !!a.archived }),
                                    )
                                  }
                                >
                                  {a.archived ? <ArchiveRestore /> : <Archive />}
                                </IconBtn>
                                <IconBtn
                                  title="Remove account"
                                  onClick={() => dispatch(setConfirm({ kind: "remove", email }))}
                                >
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
              <p
                className="shrink-0 truncate px-1 text-xs text-muted-foreground"
                title={status}
              >
                {anyBusy ? "Working… " : ""}
                {updateProgress !== null ? `Downloading update ${updateProgress}%… ` : ""}
                {status}
              </p>
              <div className="flex shrink-0 flex-wrap items-center gap-1">
                <IconBtn title="Copy status" onClick={() => void navigator.clipboard.writeText(status)}>
                  <Copy />
                </IconBtn>
                <IconBtn
                  title={logsOpen ? "Hide logs" : "Show logs"}
                  busyKey="logs:toggle"
                  onClick={() => void dispatch(toggleLogs())}
                >
                  <ScrollText />
                </IconBtn>
                {logsOpen && (
                  <IconBtn
                    title="Clear logs"
                    busyKey="logs:clear"
                    onClick={() => void dispatch(clearLogs())}
                  >
                    <Eraser />
                  </IconBtn>
                )}
                <IconBtn title="Export data" busyKey="data:export" onClick={() => void pickExportFile()}>
                  <Download />
                </IconBtn>
                <IconBtn title="Import data" busyKey="data:import" onClick={() => void pickImportFile()}>
                  <Upload />
                </IconBtn>
                <IconBtn title="Add account via Codex login" onClick={startLoginUi}>
                  <UserPlus />
                </IconBtn>
                <span className="flex items-center gap-1.5 pl-1">
                  <Label htmlFor="auto-fetch" className="text-xs text-muted-foreground">
                    Auto
                  </Label>
                  <Select
                    value={snap?.auto_fetch ?? "None"}
                    onValueChange={(v) => void dispatch(saveAutoFetchValue(v ?? "None"))}
                  >
                    <SelectTrigger
                      id="auto-fetch"
                      className="h-8 w-24"
                      title="Auto-fetch interval for the active account"
                    >
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
                <IconBtn
                  title="Fetch all accounts"
                  busyKey="fetch:all-accounts"
                  onClick={() => void dispatch(fetchAllAccounts())}
                >
                  <RefreshCw />
                </IconBtn>
                <IconBtn
                  title={snap?.show_archived ? "Hide archived accounts" : "Show archived accounts"}
                  onClick={() => void dispatch(toggleArchivedVisibility())}
                >
                  {snap?.show_archived ? <EyeOff /> : <Eye />}
                </IconBtn>
                <IconBtn
                  title="Sign out"
                  onClick={() => void dispatch(logout())}
                >
                  <LogOut />
                </IconBtn>
                <IconBtn
                  title="Check for updates"
                  busyKey="update:check"
                  onClick={() => void dispatch(checkUpdates(true))}
                >
                  <Check />
                </IconBtn>
                {updateReady && (
                  <IconBtn
                    title={updateProgress !== null ? `Installing… ${updateProgress}%` : updateReady}
                    busyKey="update:install"
                    onClick={() => void dispatch(installUpdate())}
                  >
                    <ArrowDownToLine />
                  </IconBtn>
                )}
                <IconBtn title="Toggle theme" onClick={toggle}>
                  {dark ? <Sun /> : <Moon />}
                </IconBtn>
              </div>
              {logsOpen && (
                <ScrollArea className="min-h-0 flex-1 rounded-md border bg-muted/30">
                  <pre className="whitespace-pre-wrap p-2 text-xs text-muted-foreground">
                    {logs.length === 0 ? "(no log entries yet)" : logs.join("\n")}
                  </pre>
                </ScrollArea>
              )}
            </div>
          </ResizablePanel>
        </ResizablePanelGroup>

        <Dialog open={confirm !== null} onOpenChange={(o) => !o && dispatch(setConfirm(null))}>
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
              <Button variant="outline" onClick={() => dispatch(setConfirm(null))}>
                Cancel
              </Button>
              <Button onClick={() => void dispatch(runConfirmAction())} disabled={confirmBusy}>
                {confirmBusy ? <Spin /> : "Confirm"}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog
          open={resetsEmail !== null}
          onOpenChange={(o) => !o && dispatch(setResetsView({ email: null, data: null }))}
        >
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
              if (!loginDone) void dispatch(cancelLogin());
              dispatch(setLoginOpen(false));
            }
          }}
        >
          <DialogContent className="max-w-xl">
            <DialogHeader>
              <DialogTitle>Add account — Codex login</DialogTitle>
            </DialogHeader>
            <ScrollArea className="h-56 w-full min-w-0 overflow-hidden rounded-md border bg-slate-950">
              <pre className="max-w-full whitespace-pre-wrap break-all p-2 text-xs text-slate-200 select-text">
                {loginLines.length === 0 ? "Starting login…" : loginLines.join("\n")}
              </pre>
            </ScrollArea>
            {loginUrl && !loginDone && (
              <div className="flex w-full min-w-0 flex-wrap items-center gap-2">
                <p className="w-full min-w-0 truncate text-xs text-muted-foreground" title={loginUrl}>
                  Login page ready — copy it into a private window, or open it directly.
                </p>
                <Button variant="outline" onClick={() => void dispatch(copyLoginUrl(loginUrl))}>
                  <Copy /> Copy URL
                </Button>
                <Button
                  variant="outline"
                  onClick={() => void dispatch(openLoginUrlOrDefault(loginUrl))}
                >
                  Open browser
                </Button>
              </div>
            )}
            <DialogFooter>
              {loginDone ? (
                <Button onClick={() => dispatch(setLoginOpen(false))}>Close</Button>
              ) : (
                <Button variant="outline" onClick={() => void dispatch(cancelLogin())}>
                  Cancel
                </Button>
              )}
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog open={switchInfo !== null} onOpenChange={(o) => !o && dispatch(setSwitchInfo(null))}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Switched account</DialogTitle>
              <DialogDescription>
                Now using {switchInfo}. Restart the Codex app so it picks up the new auth?
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button variant="outline" onClick={() => dispatch(setSwitchInfo(null))}>
                Later
              </Button>
              <Button
                onClick={() => {
                  void api
                    .restartCodex()
                    .then((m) => dispatch(setStatus(m)))
                    .catch((e) => dispatch(setStatus(`Restart failed: ${e}`)));
                  dispatch(setSwitchInfo(null));
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
