import {
  createAsyncThunk,
  createSlice,
  type PayloadAction,
  type ThunkDispatch,
  type UnknownAction,
} from "@reduxjs/toolkit";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api } from "../lib/tauri";
import { refreshLogs } from "./logsSlice";
import type { ResetCreditsPayload } from "../lib/format";
import type { RootState } from "./store";

function friendlyUpdateError(e: unknown): string | null {
  const raw = `${e}`;
  if (/fallback platforms|not found in the response/i.test(raw)) {
    return "Update feed is incomplete right now (a release may still be publishing) — try again in a few minutes.";
  }
  return null;
}

async function logUpdateEvent(
  dispatch: ThunkDispatch<RootState, unknown, UnknownAction>,
  message: string,
) {
  try {
    await api.logMessage(message);
  } catch {
  }
  try {
    await dispatch(refreshLogs());
  } catch {
  }
}

export type ConfirmAction =
  | { kind: "remove"; email: string }
  | { kind: "archive"; email: string; archived: boolean }
  | { kind: "switch"; email: string }
  | null;

interface UiState {
  status: string;
  busy: Record<string, boolean>;
  updateReady: string | null;
  checkingUpdate: boolean;
  updateProgress: number | null;
  confirm: ConfirmAction;
  resetsEmail: string | null;
  resets: ResetCreditsPayload | null;
  switchInfo: string | null;
}

const initialState: UiState = {
  status: "Starting…",
  busy: {},
  updateReady: null,
  checkingUpdate: false,
  updateProgress: null,
  confirm: null,
  resetsEmail: null,
  resets: null,
  switchInfo: null,
};

export const checkUpdates = createAsyncThunk<void, boolean, { state: RootState }>(
  "ui/checkUpdates",
  async (manual, { dispatch, getState }) => {
    const current = getState().snapshot.snap?.app_version || "unknown";
    if (manual) {
      dispatch(setChecking(true));
      dispatch(setBusy({ key: "update:check", on: true }));
      await logUpdateEvent(dispatch, `Checking for updates (current v${current})…`);
    }
    try {
      const update = await check();
      if (update) {
        dispatch(setUpdateReady(`Update ${update.version} ready to install.`));
        dispatch(setStatus(`Update ${update.version} is available (you're on v${current}).`));
        if (!manual) {
          await logUpdateEvent(
            dispatch,
            `Update check: v${update.version} available (current v${current}).`,
          );
        }
        if (manual) {
          dispatch(setBusy({ key: "update:install", on: true }));
          try {
            let downloaded = 0;
            let total: number | undefined;
            await update.downloadAndInstall((ev) => {
              if (ev.event === "Started") {
                total = ev.data.contentLength ?? undefined;
                dispatch(setUpdateProgress(0));
              } else if (ev.event === "Progress") {
                downloaded += ev.data.chunkLength;
                if (total) {
                  dispatch(setUpdateProgress(Math.min(99, Math.round((downloaded / total) * 100))));
                }
              } else if (ev.event === "Finished") {
                dispatch(setUpdateProgress(null));
                dispatch(setStatus(`Update ${update.version} installed, restarting…`));
              }
            });
            await logUpdateEvent(dispatch, `Update ${update.version} installed, restarting…`);
            await relaunch();
          } finally {
            dispatch(setBusy({ key: "update:install", on: false }));
          }
        }
      } else {
        dispatch(setUpdateReady(null));
        if (manual) {
          dispatch(setStatus(`You're on v${current} — the latest version.`));
        } else {
          await logUpdateEvent(dispatch, `Update check: v${current} is the latest.`);
        }
      }
    } catch (e) {
      await logUpdateEvent(dispatch, `Update check failed (current v${current}): ${e}`);
      if (manual) dispatch(setStatus(friendlyUpdateError(e) ?? `Update check failed: ${e}`));
    } finally {
      if (manual) {
        dispatch(setChecking(false));
        dispatch(setBusy({ key: "update:check", on: false }));
      }
    }
  },
);

export const installUpdate = createAsyncThunk<void, void, { state: RootState }>(
  "ui/installUpdate",
  async (_, { dispatch, getState }) => {
    const current = getState().snapshot.snap?.app_version || "unknown";
    dispatch(setBusy({ key: "update:install", on: true }));
    try {
      const update = await check();
      if (!update) {
        dispatch(setStatus(`You're on v${current} — the latest version.`));
        await logUpdateEvent(dispatch, `Install requested but v${current} is the latest.`);
        return;
      }
      await logUpdateEvent(
        dispatch,
        `Downloading update v${update.version} (current v${current})…`,
      );
      await update.downloadAndInstall((ev) => {
        if (ev.event === "Finished")
          dispatch(setStatus(`Update ${update.version} installed, restarting…`));
      });
      await logUpdateEvent(dispatch, `Update ${update.version} installed, restarting…`);
      await relaunch();
    } catch (e) {
      await logUpdateEvent(dispatch, `Update failed (current v${current}): ${e}`);
      dispatch(setStatus(friendlyUpdateError(e) ?? `Update failed: ${e}`));
    } finally {
      dispatch(setBusy({ key: "update:install", on: false }));
    }
  },
);

const uiSlice = createSlice({
  name: "ui",
  initialState,
  reducers: {
    setStatus(state, action: PayloadAction<string>) {
      state.status = action.payload;
    },
    setBusy(state, action: PayloadAction<{ key: string; on: boolean }>) {
      if (action.payload.on) state.busy[action.payload.key] = true;
      else delete state.busy[action.payload.key];
    },
    setUpdateReady(state, action: PayloadAction<string | null>) {
      state.updateReady = action.payload;
    },
    setChecking(state, action: PayloadAction<boolean>) {
      state.checkingUpdate = action.payload;
    },
    setUpdateProgress(state, action: PayloadAction<number | null>) {
      state.updateProgress = action.payload;
    },
    setConfirm(state, action: PayloadAction<ConfirmAction>) {
      state.confirm = action.payload;
    },
    setResetsView(
      state,
      action: PayloadAction<{ email: string | null; data: ResetCreditsPayload | null }>,
    ) {
      state.resetsEmail = action.payload.email;
      state.resets = action.payload.data;
    },
    setSwitchInfo(state, action: PayloadAction<string | null>) {
      state.switchInfo = action.payload;
    },
  },
});

export const {
  setStatus,
  setBusy,
  setUpdateReady,
  setChecking,
  setUpdateProgress,
  setConfirm,
  setResetsView,
  setSwitchInfo,
} = uiSlice.actions;

export const selectBusy = (key: string) => (s: RootState) => !!s.ui.busy[key];
export const selectAnyBusy = (s: RootState) => Object.keys(s.ui.busy).length > 0;

export default uiSlice.reducer;
