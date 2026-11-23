import {
  createContext,
  useContext,
  useMemo,
  useSyncExternalStore,
  type ReactNode,
} from "react";

import {
  getAppearance,
  getResolvedTheme,
  setAppearance,
  subscribeAppearance,
  type AppearancePreference,
  type ResolvedTheme,
} from "@/lib/appearance";

interface AppearanceContextValue {
  appearance: AppearancePreference;
  resolvedTheme: ResolvedTheme;
  setAppearance: (appearance: AppearancePreference) => void;
}

const AppearanceContext = createContext<AppearanceContextValue | null>(null);

export function AppearanceProvider({ children }: { children: ReactNode }) {
  const appearance = useSyncExternalStore(
    subscribeAppearance,
    getAppearance,
    () => "system" as const,
  );
  const resolvedTheme = useSyncExternalStore(
    subscribeAppearance,
    getResolvedTheme,
    () => "dark" as const,
  );
  const value = useMemo(
    () => ({ appearance, resolvedTheme, setAppearance }),
    [appearance, resolvedTheme],
  );

  return (
    <AppearanceContext.Provider value={value}>
      {children}
    </AppearanceContext.Provider>
  );
}

export function useAppearance(): AppearanceContextValue {
  const value = useContext(AppearanceContext);
  if (!value) {
    throw new Error("useAppearance must be used within AppearanceProvider");
  }
  return value;
}
