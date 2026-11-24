import { createAsyncThunk, createSlice } from "@reduxjs/toolkit";
import { openUrl } from "@tauri-apps/plugin-opener";
import { translateFor } from "../i18n/messages";
import { api } from "../lib/tauri";
import { setBusy, setStatus } from "./uiSlice";
import type { RootState } from "./store";

interface LoginState {
  open: boolean;
  lines: string[];
  done: string | null;
  starting: boolean;
  urlOpened: boolean;
  url: string | null;
  error: string | null;
}

const initialState: LoginState = {
  open: false,
  lines: [],
  done: null,
  starting: false,
  urlOpened: false,
  url: null,
  error: null,
};

export const openLoginUrlOrDefault = createAsyncThunk<void, string, { state: RootState }>(
  "login/openUrl",
  async (url, { dispatch, getState }) => {
    try {
      dispatch(setStatus(await api.openLoginUrl(url)));
    } catch {
      dispatch(
        setStatus(
          translateFor(getState().snapshot.snap?.locale, "status.openLoginFallback"),
        ),
      );
      void openUrl(url).catch(() => undefined);
    }
  },
);

export const copyLoginUrl = createAsyncThunk<void, string, { state: RootState }>(
  "login/copyUrl",
  async (url, { dispatch, getState }) => {
    const preference = getState().snapshot.snap?.locale;
    try {
      await navigator.clipboard.writeText(url);
      dispatch(setStatus(translateFor(preference, "status.loginUrlCopied")));
    } catch {
      dispatch(setStatus(translateFor(preference, "status.loginUrlCopyFailed")));
    }
  },
);

export const startLogin = createAsyncThunk<void, void, { state: RootState }>(
  "login/start",
  async (_, { getState, dispatch }) => {
    if (getState().login.starting) return;
    dispatch(setStarting(true));
    dispatch(resetLines());
    dispatch(setOpen(true));
    try {
      const msg = await api.loginStart();
      dispatch(setStatus(msg));
    } catch (e) {
      dispatch(setError(`${e}`));
      dispatch(
        setStatus(
          translateFor(getState().snapshot.snap?.locale, "status.signInStartFailed", {
            error: `${e}`,
          }),
        ),
      );
    } finally {
      dispatch(setStarting(false));
    }
  },
);

export const cancelLogin = createAsyncThunk<void, void, { state: RootState }>(
  "login/cancel",
  async (_, { dispatch, getState }) => {
    dispatch(setBusy({ key: "login:cancel", on: true }));
    try {
      await api.loginCancel();
    } catch (e) {
      dispatch(
        setStatus(
          translateFor(getState().snapshot.snap?.locale, "status.cancelFailed", {
            error: `${e}`,
          }),
        ),
      );
    } finally {
      dispatch(setBusy({ key: "login:cancel", on: false }));
    }
  },
);

const loginSlice = createSlice({
  name: "login",
  initialState,
  reducers: {
    setOpen(state, action: { payload: boolean }) {
      state.open = action.payload;
    },
    setStarting(state, action: { payload: boolean }) {
      state.starting = action.payload;
    },
    resetLines(state) {
      state.lines = [];
      state.done = null;
      state.urlOpened = false;
      state.url = null;
      state.error = null;
    },
    setError(state, action: { payload: string | null }) {
      state.error = action.payload;
    },
    setUrl(state, action: { payload: string }) {
      state.url = action.payload;
    },
    appendLines(state, action: { payload: string[] }) {
      for (const line of action.payload) {
        if (line !== state.lines[state.lines.length - 1]) {
          state.lines.push(line);
        }
      }
      state.lines = state.lines.slice(-200);
    },
    markUrlOpened(state) {
      state.urlOpened = true;
    },
    setDone(state, action: { payload: string }) {
      state.done = action.payload;
    },
  },
});

export const {
  setOpen,
  setStarting,
  resetLines,
  appendLines,
  markUrlOpened,
  setDone,
  setUrl,
  setError,
} = loginSlice.actions;
export default loginSlice.reducer;
