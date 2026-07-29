import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

export type DesktopPanel = "connection" | "diagnostics";

export interface UiState {
  activePanel: DesktopPanel;
  diagnosticsExpanded: boolean;
}

const initialState: UiState = {
  activePanel: "connection",
  diagnosticsExpanded: false,
};

export const uiSlice = createSlice({
  name: "ui",
  initialState,
  reducers: {
    activePanelChanged(state, action: PayloadAction<DesktopPanel>) {
      state.activePanel = action.payload;
    },
    diagnosticsExpandedChanged(state, action: PayloadAction<boolean>) {
      state.diagnosticsExpanded = action.payload;
    },
  },
});

export const uiActions = uiSlice.actions;
export default uiSlice.reducer;
