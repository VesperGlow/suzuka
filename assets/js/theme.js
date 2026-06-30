(() => {
  const storageKey = "suzuka-theme";
  const root = document.documentElement;
  const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const transitionDuration = 700;
  let transitionTimer;

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

  const applyThemeFromControl = (theme) => {
    window.clearTimeout(transitionTimer);

    if (reducedMotion.matches) {
      root.classList.remove("theme-transitioning");
    } else {
      root.classList.add("theme-transitioning");
      // Commit the transition rules before changing the theme variables.
      void root.offsetWidth;
    }

    applyTheme(theme, true);

    if (!reducedMotion.matches) {
      transitionTimer = window.setTimeout(() => {
        root.classList.remove("theme-transitioning");
      }, transitionDuration);
    }
  };

  const declaredTheme = root.dataset.theme;
  applyTheme(
    declaredTheme === "dark" || declaredTheme === "light"
      ? declaredTheme
      : getSavedTheme()
  );

  // Delegate on document so the toggle responds the moment it is parsed,
  // without waiting for DOMContentLoaded (and the deferred scripts it blocks on).
  document.addEventListener("click", (event) => {
    const toggle =
      event.target instanceof Element
        ? event.target.closest("[data-theme-toggle]")
        : null;
    if (!toggle) return;
    applyThemeFromControl(getResolvedTheme() === "dark" ? "light" : "dark");
  });

  document.addEventListener("DOMContentLoaded", updateControls);

  colorScheme.addEventListener("change", () => {
    if (!root.dataset.theme) updateControls();
  });
})();
