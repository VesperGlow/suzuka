(() => {
  const storageKey = "suzuka-theme";
  const root = document.documentElement;
  const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");

  const getSavedTheme = () => {
    try {
      const theme = localStorage.getItem(storageKey);
      return theme === "dark" || theme === "light" ? theme : null;
    } catch {
      return null;
    }
  };

  const getResolvedTheme = () =>
    root.dataset.theme || (colorScheme.matches ? "dark" : "light");

  const updateControls = () => {
    const isDark = getResolvedTheme() === "dark";
    const themeColor = document.querySelector("#theme-color");

    if (themeColor) themeColor.content = isDark ? "#0e1626" : "#f5f9fc";
    document.querySelectorAll("[data-theme-toggle]").forEach((button) => {
      const label = isDark ? button.dataset.labelLight : button.dataset.labelDark;
      button.setAttribute("aria-label", label);
      button.setAttribute("title", label);
      button.setAttribute("aria-pressed", String(isDark));
    });
  };

  const applyTheme = (theme, persist = false) => {
    if (theme) root.dataset.theme = theme;
    else delete root.dataset.theme;

    if (persist) {
      try {
        localStorage.setItem(storageKey, theme);
      } catch {
        // The selected theme still applies for the current page.
      }
    }

    updateControls();
  };

  const declaredTheme = root.dataset.theme;
  applyTheme(
    declaredTheme === "dark" || declaredTheme === "light"
      ? declaredTheme
      : getSavedTheme()
  );

  document.addEventListener("DOMContentLoaded", () => {
    updateControls();
    document.querySelectorAll("[data-theme-toggle]").forEach((button) => {
      button.addEventListener("click", () => {
        applyTheme(getResolvedTheme() === "dark" ? "light" : "dark", true);
      });
    });
  });

  colorScheme.addEventListener("change", () => {
    if (!root.dataset.theme) updateControls();
  });
})();
