document.querySelectorAll("[data-post-filter-scope]").forEach((scope) => {
  const tagToggle = scope.querySelector(".archive-tag-toggle");
  const tagPanel = scope.querySelector(".archive-tag-panel");
  const buttons = [...scope.querySelectorAll("[data-filter-type]")];
  const items = [...scope.querySelectorAll("[data-filter-item]")];
  const groups = [...scope.querySelectorAll("[data-filter-group]")];
  const empty = scope.querySelector("[data-filter-empty]");
  const viewButtons = [...scope.querySelectorAll("[data-archive-view]")];
  const viewPanels = [...scope.querySelectorAll("[data-archive-view-panel]")];

  const archiveViews = ["cards", "timeline"];

  function isArchiveView(view) {
    return archiveViews.includes(view);
  }

  function viewFromHash() {
    const view = location.hash.slice(1);
    return isArchiveView(view) ? view : null;
  }

  function storedArchiveView() {
    try {
      const view = localStorage.getItem("archiveView");
      return isArchiveView(view) ? view : null;
    } catch {
      return null;
    }
  }

  function getInitialArchiveView() {
    const earlyView = document.documentElement.dataset.archiveView;
    return viewFromHash() || (isArchiveView(earlyView) ? earlyView : null) || storedArchiveView() || "cards";
  }

  function setArchiveView(view, options = {}) {
    if (!isArchiveView(view)) return;

    const { store = true, updateHash = true } = options;

    document.documentElement.dataset.archiveView = view;

    viewButtons.forEach((button) => {
      const active = button.dataset.archiveView === view;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    });
    viewPanels.forEach((panel) => {
      panel.hidden = panel.dataset.archiveViewPanel !== view;
    });

    if (store) {
      try {
        localStorage.setItem("archiveView", view);
      } catch {
        // The view still works when storage is unavailable.
      }
    }

    const hash = `#${view}`;
    if (updateHash && location.hash !== hash) {
      history.replaceState(null, "", hash);
    }
  }

  if (viewButtons.length && viewPanels.length) {
    viewButtons.forEach((button) => {
      button.addEventListener("click", () => {
        setArchiveView(button.dataset.archiveView);
      });
    });

    window.addEventListener("hashchange", () => {
      const view = viewFromHash();
      if (view) setArchiveView(view, { updateHash: false });
    });

    setArchiveView(getInitialArchiveView(), { store: false, updateHash: false });
    requestAnimationFrame(() => {
      requestAnimationFrame(() => document.documentElement.classList.remove("archive-no-transition"));
    });
  }

  if (tagToggle && tagPanel) {
    tagToggle.addEventListener("click", () => {
      const expanded = tagToggle.getAttribute("aria-expanded") === "true";
      tagToggle.setAttribute("aria-expanded", String(!expanded));
      tagPanel.classList.toggle("is-expanded", !expanded);
    });
  }

  if (!buttons.length || !items.length) return;

  function valuesFor(item, type) {
    try {
      return JSON.parse(item.dataset[type] || "[]");
    } catch {
      return [];
    }
  }

  function applyFilter(type, value) {
    let visibleCount = 0;

    items.forEach((item) => {
      const visible = type === "all" || valuesFor(item, type).includes(value);
      item.hidden = !visible;
      item.classList.toggle("is-hidden", !visible);
      if (visible) visibleCount += 1;
    });

    groups.forEach((group) => {
      const visible = Boolean(group.querySelector("[data-filter-item]:not([hidden])"));
      group.hidden = !visible;
      group.classList.toggle("is-hidden", !visible);
    });

    if (empty) {
      const visible = visibleCount === 0;
      empty.hidden = !visible;
      empty.classList.toggle("is-hidden", !visible);
    }

    buttons.forEach((button) => {
      const active = button.dataset.filterType === type && button.dataset.filterValue === value;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  buttons.forEach((button) => {
    button.addEventListener("click", () => {
      applyFilter(button.dataset.filterType, button.dataset.filterValue);
    });
  });

  const initial = buttons.find((button) => button.classList.contains("active")) || buttons[0];
  applyFilter(initial.dataset.filterType, initial.dataset.filterValue);
});
