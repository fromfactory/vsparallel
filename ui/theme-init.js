(function () {
  "use strict";

  const storageKey = "vsparallel.appearance";
  const supportedPreferences = new Set(["system", "light", "dark"]);
  let preference = "system";

  try {
    const storedPreference = window.localStorage.getItem(storageKey);
    if (supportedPreferences.has(storedPreference)) {
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
