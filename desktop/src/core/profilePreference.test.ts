import { beforeEach, describe, expect, it } from "vitest";

import {
  DEFAULT_PROFILE_ID,
  isValidProfileIdPreference,
  readProfileIdPreference,
  saveProfileIdPreference,
} from "./profilePreference";

describe("profile ID preference", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("defaults to the existing main profile and persists a selected ID", () => {
    expect(readProfileIdPreference()).toBe(DEFAULT_PROFILE_ID);

    saveProfileIdPreference("studio_2");

    expect(readProfileIdPreference()).toBe("studio_2");
  });

  it.each(["a", "main", "oakland-night_2", "z".repeat(64)])(
    "accepts valid profile ID %s",
    (value) => {
      expect(isValidProfileIdPreference(value)).toBe(true);
    },
  );

  it.each(["", " Main", "Main", "-main", "main-", "a".repeat(65), "space name"])(
    "rejects invalid profile ID %s",
    (value) => {
      expect(isValidProfileIdPreference(value)).toBe(false);
    },
  );
});
