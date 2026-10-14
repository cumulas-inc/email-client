import { useEffect } from "react";

import { useSettings } from "../api/queries";

/** Applies the theme setting to <html data-theme>. */
export function useTheme(): void {
  const { data: settings } = useSettings();
  const theme = settings?.theme ?? "system";
  useEffect(() => {
    const root = document.documentElement;
    if (theme === "system") delete root.dataset.theme;
    else root.dataset.theme = theme;
  }, [theme]);
}
