import { configureStore } from "@reduxjs/toolkit";
import login from "./loginSlice";
import logs from "./logsSlice";
import snapshot from "./snapshotSlice";
import ui from "./uiSlice";

export const store = configureStore({
  reducer: { ui, snapshot, logs, login },
});

export type RootState = ReturnType<typeof store.getState>;
export type AppDispatch = typeof store.dispatch;
