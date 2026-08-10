import { describe, expect, it } from "vitest";

import labReducer, { type LabState, labActions } from "./labSlice";

describe("labSlice", () => {
  it("starts with availability unknown, not assumed false", () => {
    const state = labReducer(undefined, { type: "@@INIT" });
    expect(state.available).toBeNull();
  });

  it("records the backend's own availability answer", () => {
    const initial: LabState = { available: null };
    const afterTrue = labReducer(initial, labActions.labModeAvailabilityReceived(true));
    expect(afterTrue.available).toBe(true);

    const afterFalse = labReducer(afterTrue, labActions.labModeAvailabilityReceived(false));
    expect(afterFalse.available).toBe(false);
  });
});
