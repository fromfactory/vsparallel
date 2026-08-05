(function () {
  "use strict";

  type ThemePreference = "system" | "light" | "dark";

  const storageKey = "vsparallel.appearance";
  const supportedPreferences: ReadonlySet<string> = new Set(["system", "light", "dark"]);
  let preference: ThemePreference = "system";

  function isThemePreference(value: unknown): value is ThemePreference {
    return typeof value === "string" && supportedPreferences.has(value);
  }

  try {
    const storedPreference = window.localStorage.getItem(storageKey);
    if (isThemePreference(storedPreference)) {
      preference = storedPreference;
    }
  } catch (_error) {
    // The system preference remains a safe default when storage is unavailable.
  }

  const systemTheme = window.matchMedia("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
  document.documentElement.dataset.themePreference = preference;
  document.documentElement.dataset.colorTheme = preference === "system"
    ? systemTheme
    : preference;
})();
