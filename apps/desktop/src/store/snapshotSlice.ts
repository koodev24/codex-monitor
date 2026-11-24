import { createAsyncThunk, createSlice } from "@reduxjs/toolkit";
import { sendNotification } from "@tauri-apps/plugin-notification";
import { translateFor, type MessageKey } from "../i18n/messages";
import { api, type Snapshot } from "../lib/tauri";
import { refreshLogs } from "./logsSlice";
import { setBusy, setConfirm, setResetsView, setStatus, setSwitchInfo } from "./uiSlice";
import type { RootState } from "./store";

export type SortKey = "email" | "quota" | "reset";

interface SnapshotState {
  snap: Snapshot | null;
  initialized: boolean;
  sortKey: SortKey | null;
  sortAsc: boolean;
  authRetries: number;
  loadError: string | null;
  fetchFailed: Record<string, string>;
}

const initialState: SnapshotState = {
  snap: null,
  initialized: false,
  sortKey: null,
  sortAsc: true,
  authRetries: 0,
  loadError: null,
  fetchFailed: {},
};

function isAuthFetchFailure(message: string): boolean {
  return /401|expired|unauthori[sz]ed|no saved|no access token/i.test(message);
}

const MISSING_TOKEN_RETRIES = 6;

function sortFromColumn(col: string | null): SortKey | null {
  if (col === "weekly_quota" || col === "quota") return "quota";
  if (col === "weekly_reset") return "reset";
  if (col === "email") return "email";
  return null;
}

export const loadSnapshot = createAsyncThunk<void, void, { state: RootState }>(
  "snapshot/load",
  async (_, { dispatch, getState }) => {
    let lastError = "unknown error";
    for (let attempt = 1; attempt <= 3; attempt++) {
      try {
        const s = await api.snapshot();
        dispatch(setSnapshot(s));
        dispatch(setLoadError(null));
        const status = getState().ui.status;
        if (
          status === "Starting…" ||
          status === translateFor(s.locale, "status.starting")
        ) {
          dispatch(setStatus(translateFor(s.locale, "status.ready")));
        }
        return;
      } catch (e) {
        lastError = `${e}`;
        if (attempt < 3) await new Promise((r) => setTimeout(r, 800));
      }
    }
    dispatch(setLoadError(lastError));
    dispatch(
      setStatus(
        translateFor(getState().snapshot.snap?.locale, "status.loadFailed", {
          error: lastError,
        }),
      ),
    );
  },
);

export const pollOnce = createAsyncThunk<void, void, { state: RootState }>(
  "snapshot/poll",
  async (_, { getState, dispatch }) => {
    const t = (key: MessageKey, params?: Record<string, string | number>) =>
      translateFor(getState().snapshot.snap?.locale, key, params);
    try {
      const o = await api.processAuthFile();
      if (o.kind === "NoChange") {
        // fall through to auto-fetch check
      } else if (o.kind === "MissingToken") {
        const retries = getState().snapshot.authRetries;
        if (retries < MISSING_TOKEN_RETRIES) {
          dispatch(setAuthRetries(retries + 1));
          dispatch(setStatus(t("status.authChanging")));
          setTimeout(() => void dispatch(pollOnce()), 350);
          return;
        }
        dispatch(setAuthRetries(0));
        await dispatch(loadSnapshot());
        await dispatch(refreshLogs());
        return;
      } else {
        dispatch(setAuthRetries(0));
        if (o.kind === "AuthRefreshed") {
          dispatch(setStatus(o.message));
          if (getState().snapshot.snap?.notifications_enabled !== false) {
            try {
              sendNotification({ title: t("notifications.title"), body: o.message });
            } catch {
            }
          }
        } else if (o.kind === "Fetched" || o.kind === "LoggedOut") {
          dispatch(setStatus(o.message));
        } else if (o.kind === "ParseError" || o.kind === "NoFile") {
          dispatch(setStatus(o.kind === "NoFile" ? t("status.authRemoved") : o.message));
        }
        await dispatch(loadSnapshot());
        await dispatch(refreshLogs());
      }
    } catch {
      /* transient */
    }
    try {
      const r = await api.checkAutoFetch();
      if (r) {
        dispatch(setStatus(r.message));
        await dispatch(loadSnapshot());
        await dispatch(refreshLogs());
      }
    } catch (e) {
      dispatch(setStatus(t("status.autoFetchFailed", { error: `${e}` })));
    }
  },
);

export const manualFetch = createAsyncThunk<void, string | undefined, { state: RootState }>(
  "snapshot/manualFetch",
  async (email, { dispatch, getState }) => {
    const t = (key: MessageKey, params?: Record<string, string | number>) =>
      translateFor(getState().snapshot.snap?.locale, key, params);
    const key = email ? `fetch:${email}` : "fetch:all";
    if (getState().ui.busy[key]) return;
    dispatch(setBusy({ key, on: true }));
    dispatch(
      setStatus(email ? t("status.fetchingAccount", { email }) : t("status.fetchingQuota")),
    );
    try {
      const r = email ? await api.fetchBackup(email) : await api.manualFetch();
      dispatch(setStatus(r.message));
      dispatch(clearFetchFailed(email ?? getState().snapshot.snap?.current_email ?? ""));
      await dispatch(loadSnapshot());
      await dispatch(refreshLogs());
    } catch (e) {
      const raw = `${e}`;
      const prefix = email ? `NO_BACKUP ${email}: ` : null;
      const message = prefix && raw.startsWith(prefix) ? raw.slice(prefix.length) : raw;
      dispatch(setStatus(t("status.fetchFailed", { error: message })));
      const target = email ?? getState().snapshot.snap?.current_email;
      if (target && isAuthFetchFailure(message)) {
        dispatch(setFetchFailed({ email: target, reason: message }));
      }
    } finally {
      dispatch(setBusy({ key, on: false }));
    }
  },
);

export const fetchAllAccounts = createAsyncThunk<void, void, { state: RootState }>(
  "snapshot/fetchAll",
  async (_, { dispatch, getState }) => {
    const t = (key: MessageKey, params?: Record<string, string | number>) =>
      translateFor(getState().snapshot.snap?.locale, key, params);
    const KEY = "fetch:all-accounts";
    if (getState().ui.busy[KEY]) return;
    const snap = getState().snapshot.snap;
    const emails = snap ? Object.keys(snap.accounts) : [];
    if (emails.length === 0) {
      dispatch(setStatus(t("status.noAccountsToFetch")));
      return;
    }
    const current = snap?.current_email ?? null;
    const rowKeyOf = (account: string) =>
      account === current ? "fetch:all" : `fetch:${account}`;
    dispatch(setBusy({ key: KEY, on: true }));
    emails.forEach((account) => dispatch(setBusy({ key: rowKeyOf(account), on: true })));
    dispatch(setStatus(t("status.fetchingCount", { count: emails.length })));
    try {
      const results = await Promise.allSettled(
        emails.map((account) =>
          account === current ? api.manualFetch() : api.fetchBackup(account),
        ),
      );
      let fetched = 0;
      const failed: string[] = [];
      results.forEach((r, i) => {
        if (r.status === "fulfilled") {
          fetched++;
          dispatch(clearFetchFailed(emails[i]));
        } else {
          const raw = `${r.reason}`;
          const prefix = `NO_BACKUP ${emails[i]}: `;
          const message = raw.startsWith(prefix) ? raw.slice(prefix.length) : raw;
          failed.push(`${emails[i]} (${message})`);
          if (isAuthFetchFailure(message)) {
            dispatch(setFetchFailed({ email: emails[i], reason: message }));
          }
        }
      });
      await dispatch(loadSnapshot());
      await dispatch(refreshLogs());
      if (failed.length === 0) {
        dispatch(setStatus(t("status.fetchedAll", { count: fetched })));
      } else {
        dispatch(
          setStatus(
            t("status.fetchedPartial", {
              fetched,
              failed: failed.length,
              errors: failed.join("; "),
            }),
          ),
        );
      }
    } finally {
      emails.forEach((account) => dispatch(setBusy({ key: rowKeyOf(account), on: false })));
      dispatch(setBusy({ key: KEY, on: false }));
    }
  },
);

export const runConfirmAction = createAsyncThunk<void, void, { state: RootState }>(
  "snapshot/confirm",
  async (_, { getState, dispatch }) => {
    const t = (key: MessageKey, params?: Record<string, string | number>) =>
      translateFor(getState().snapshot.snap?.locale, key, params);
    const confirm = getState().ui.confirm;
    if (!confirm || getState().ui.busy["confirm"]) return;
    if (confirm.kind === "switch") {
      dispatch(setStatus(t("status.switchingTo", { email: confirm.email })));
      dispatch(setSwitchInfo(null));
    }
    dispatch(setBusy({ key: "confirm", on: true }));
    try {
      if (confirm.kind === "remove") {
        dispatch(setStatus(await api.removeAccount(confirm.email)));
        dispatch(clearFetchFailed(confirm.email));
      } else if (confirm.kind === "archive") {
        dispatch(setStatus(await api.setArchived(confirm.email, !confirm.archived)));
      } else {
        const result = await api.switchAccount(confirm.email);
        dispatch(setStatus(result.message));
        dispatch(setSwitchInfo(result));
      }
      await dispatch(loadSnapshot());
      await dispatch(refreshLogs());
    } catch (e) {
      dispatch(
        setStatus(translateFor(getState().snapshot.snap?.locale, "status.actionFailed", { error: `${e}` })),
      );
    } finally {
      dispatch(setConfirm(null));
      dispatch(setBusy({ key: "confirm", on: false }));
    }
  },
);

export const openResets = createAsyncThunk<void, string, { state: RootState }>(
  "snapshot/resets",
  async (email, { dispatch, getState }) => {
    dispatch(setBusy({ key: `resets:${email}`, on: true }));
    try {
      dispatch(setResetsView({ email, data: await api.resets(email) }));
    } catch (e) {
      dispatch(
        setStatus(
          translateFor(getState().snapshot.snap?.locale, "status.resetCreditsFailed", {
            error: `${e}`,
          }),
        ),
      );
    } finally {
      dispatch(setBusy({ key: `resets:${email}`, on: false }));
    }
  },
);

export const doExport = createAsyncThunk<void, string, { state: RootState }>(
  "snapshot/export",
  async (path, { dispatch, getState }) => {
    dispatch(setBusy({ key: "data:export", on: true }));
    try {
      dispatch(setStatus(await api.exportData(path)));
      await dispatch(refreshLogs());
    } catch (e) {
      dispatch(
        setStatus(translateFor(getState().snapshot.snap?.locale, "status.exportFailed", { error: `${e}` })),
      );
    } finally {
      dispatch(setBusy({ key: "data:export", on: false }));
    }
  },
);

export const doImport = createAsyncThunk<void, string, { state: RootState }>(
  "snapshot/import",
  async (path, { dispatch, getState }) => {
    dispatch(setBusy({ key: "data:import", on: true }));
    try {
      dispatch(setStatus(await api.importData(path)));
      await dispatch(loadSnapshot());
      await dispatch(refreshLogs());
    } catch (e) {
      dispatch(
        setStatus(translateFor(getState().snapshot.snap?.locale, "status.importFailed", { error: `${e}` })),
      );
    } finally {
      dispatch(setBusy({ key: "data:import", on: false }));
    }
  },
);

export const saveAutoFetchValue = createAsyncThunk<void, string, { state: RootState }>(
  "snapshot/autoFetch",
  async (value, { dispatch, getState }) => {
    dispatch(setBusy({ key: "auto-fetch", on: true }));
    try {
      dispatch(setStatus(await api.saveAutoFetch(value)));
      await dispatch(loadSnapshot());
      await dispatch(refreshLogs());
    } catch (e) {
      dispatch(
        setStatus(
          translateFor(getState().snapshot.snap?.locale, "status.autoFetchSaveFailed", {
            error: `${e}`,
          }),
        ),
      );
    } finally {
      dispatch(setBusy({ key: "auto-fetch", on: false }));
    }
  },
);

export const cycleSort = createAsyncThunk<void, SortKey, { state: RootState }>(
  "snapshot/sort",
  async (key, { getState, dispatch }) => {
    const { sortKey, sortAsc } = getState().snapshot;
    let nextKey: SortKey | null = key;
    let nextAsc = true;
    if (sortKey === key) {
      if (sortAsc) nextAsc = false;
      else nextKey = null;
    }
    dispatch(setSortView({ key: nextKey, asc: nextAsc }));
    const col =
      nextKey === "quota" ? "weekly_quota" : nextKey === "reset" ? "weekly_reset" : nextKey;
    try {
      await api.saveSort(col, nextAsc);
    } catch {
      /* pref only */
    }
  },
);

export const toggleArchivedVisibility = createAsyncThunk<void, void, { state: RootState }>(
  "snapshot/archivedVisibility",
  async (_, { getState, dispatch }) => {
    const next = !(getState().snapshot.snap?.show_archived ?? false);
    try {
      await api.saveShowArchived(next);
      await dispatch(loadSnapshot());
    } catch (e) {
      dispatch(
        setStatus(translateFor(getState().snapshot.snap?.locale, "status.toggleFailed", { error: `${e}` })),
      );
    }
  },
);

export type SettingsPatch = Partial<
  Pick<
    Snapshot,
    "show_menu_bar" | "notifications_enabled" | "auto_check_updates" | "locale"
  >
>;

export const saveSettings = createAsyncThunk<void, SettingsPatch, { state: RootState }>(
  "snapshot/settings",
  async (patch, { dispatch, getState }) => {
    const t = (key: MessageKey, params?: Record<string, string | number>) =>
      translateFor(getState().snapshot.snap?.locale, key, params);
    dispatch(setBusy({ key: "settings", on: true }));
    try {
      if (patch.show_menu_bar !== undefined) {
        await api.saveShowMenuBar(patch.show_menu_bar);
        dispatch(
          setStatus(
            patch.show_menu_bar ? t("status.menuBarShown") : t("status.menuBarHidden"),
          ),
        );
      }
      if (patch.notifications_enabled !== undefined) {
        await api.saveNotificationsEnabled(patch.notifications_enabled);
      }
      if (patch.auto_check_updates !== undefined) {
        await api.saveAutoCheckUpdates(patch.auto_check_updates);
      }
      if (patch.locale !== undefined) {
        await api.saveLocale(patch.locale);
      }
    } catch (e) {
      dispatch(setStatus(t("status.settingsFailed", { error: `${e}` })));
    } finally {
      await dispatch(loadSnapshot());
      dispatch(setBusy({ key: "settings", on: false }));
    }
  },
);

export const logout = createAsyncThunk<void, void, { state: RootState }>(
  "snapshot/logout",
  async (_, { dispatch, getState }) => {
    try {
      dispatch(setStatus(await api.logout()));
      await dispatch(loadSnapshot());
      await dispatch(refreshLogs());
    } catch (e) {
      dispatch(
        setStatus(translateFor(getState().snapshot.snap?.locale, "status.logoutFailed", { error: `${e}` })),
      );
    }
  },
);

const snapshotSlice = createSlice({
  name: "snapshot",
  initialState,
  reducers: {
    setSnapshot(state, action: { payload: Snapshot }) {
      state.snap = action.payload;
      state.sortKey = sortFromColumn(action.payload.sort_column);
      state.sortAsc = action.payload.sort_asc;
      state.initialized = true;
    },
    setSortView(state, action: { payload: { key: SortKey | null; asc: boolean } }) {
      state.sortKey = action.payload.key;
      state.sortAsc = action.payload.asc;
    },
    setAuthRetries(state, action: { payload: number }) {
      state.authRetries = action.payload;
    },
    setLoadError(state, action: { payload: string | null }) {
      state.loadError = action.payload;
    },
    setFetchFailed(state, action: { payload: { email: string; reason: string } }) {
      if (action.payload.email) state.fetchFailed[action.payload.email] = action.payload.reason;
    },
    clearFetchFailed(state, action: { payload: string }) {
      if (action.payload) delete state.fetchFailed[action.payload];
    },
  },
});

export const { setSnapshot, setSortView, setAuthRetries, setLoadError, setFetchFailed, clearFetchFailed } =
  snapshotSlice.actions;
export default snapshotSlice.reducer;
