const PROFILE_ID_STORAGE_KEY = "silent-disco.profile-id";
export const DEFAULT_PROFILE_ID = "main";
const PROFILE_ID_PATTERN = /^[a-z0-9](?:[a-z0-9_-]{0,62}[a-z0-9])?$/;

/// Reads the profile ID selected for the next bridge startup.
///
/// Invalid stored values are returned unchanged so the backend can report
/// its validation error and Settings can offer a way to correct the value.
export function readProfileIdPreference(): string {
  return window.localStorage.getItem(PROFILE_ID_STORAGE_KEY) ?? DEFAULT_PROFILE_ID;
}

export function isValidProfileIdPreference(value: string): boolean {
  return value.length <= 64 && PROFILE_ID_PATTERN.test(value);
}

export function saveProfileIdPreference(value: string): void {
  window.localStorage.setItem(PROFILE_ID_STORAGE_KEY, value);
}
