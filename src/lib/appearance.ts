export const APPEARANCE_STORAGE_KEY = "cvec.appearance.v2";

export type AppearancePreference = "system" | "dark" | "light";
export type ResolvedTheme = "dark" | "light";

const listeners = new Set<() => void>();
let preference: AppearancePreference = "system";
let resolvedTheme: ResolvedTheme = "dark";
let mediaQuery: MediaQueryList | null = null;
let initialized = false;

export function isAppearancePreference(
  value: unknown,
): value is AppearancePreference {
  return value === "system" || value === "dark" || value === "light";
}

export function parseAppearance(value: string | null): AppearancePreference {
  return isAppearancePreference(value) ? value : "system";
}

export function resolveAppearance(
  value: AppearancePreference,
  systemPrefersDark: boolean,
): ResolvedTheme {
  if (value === "system") {
    return systemPrefersDark ? "dark" : "light";
  }
  return value;
}

export function bootstrapAppearance(): AppearancePreference {
  if (typeof window === "undefined" || typeof document === "undefined") {
    return "system";
  }

  if (!initialized) {
    mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    mediaQuery.addEventListener("change", handleSystemAppearanceChange);
    window.addEventListener("storage", handleStorageChange);
    initialized = true;
  }

  try {
    preference = parseAppearance(
      window.localStorage.getItem(APPEARANCE_STORAGE_KEY),
    );
  } catch {
    preference = "system";
  }
  applyAppearance();
  return preference;
}

export function setAppearance(value: AppearancePreference): void {
  preference = value;
  try {
    window.localStorage.setItem(APPEARANCE_STORAGE_KEY, value);
  } catch {
    // Appearance still applies for the current session when storage is unavailable.
  }
  applyAppearance();
  emitChange();
}

export function getAppearance(): AppearancePreference {
  return preference;
}

export function getResolvedTheme(): ResolvedTheme {
  return resolvedTheme;
}

export function subscribeAppearance(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function applyAppearance(): void {
  const systemPrefersDark = mediaQuery?.matches ?? true;
  resolvedTheme = resolveAppearance(preference, systemPrefersDark);
  document.documentElement.dataset.appearance = preference;
  document.documentElement.dataset.theme = resolvedTheme;
  document.documentElement.style.colorScheme = resolvedTheme;
}

function handleSystemAppearanceChange(): void {
  if (preference !== "system") {
    return;
  }
  applyAppearance();
  emitChange();
}

function handleStorageChange(event: StorageEvent): void {
  if (event.key !== APPEARANCE_STORAGE_KEY) {
    return;
  }
  preference = parseAppearance(event.newValue);
  applyAppearance();
  emitChange();
}

function emitChange(): void {
  listeners.forEach((listener) => listener());
}
