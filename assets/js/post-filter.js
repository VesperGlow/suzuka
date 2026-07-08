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

  // hash 形如 "#cards" 或 "#cards/tags=博客"：第一段是视图，第二段是筛选。
  // 筛选值经过 encodeURIComponent，值里的 "/" 不会干扰分段。
  function parseHash() {
    const [viewPart, ...rest] = location.hash.slice(1).split("/");
    const view = isArchiveView(viewPart) ? viewPart : null;
    let filter = null;
    const filterPart = rest.join("/");
    const eq = filterPart.indexOf("=");
    if (eq > 0) {
      const type = filterPart.slice(0, eq);
      if (type === "categories" || type === "tags") {
        try {
          filter = { type, value: decodeURIComponent(filterPart.slice(eq + 1)) };
        } catch {
          /* 非法编码当没有筛选 */
        }
      }
    }
    return { view, filter };
  }

  // 当前生效的筛选，跟视图一起编进 hash：刷新/分享链接不丢筛选状态
  let activeFilter = { type: "all", value: "" };

  function currentView() {
    const view = document.documentElement.dataset.archiveView;
    return isArchiveView(view) ? view : "cards";
  }

  function syncHash() {
    if (!viewButtons.length) return;
    let hash = `#${currentView()}`;
    if (activeFilter.type !== "all") {
      hash += `/${activeFilter.type}=${encodeURIComponent(activeFilter.value)}`;
    }
    if (location.hash !== hash) history.replaceState(null, "", hash);
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
    return parseHash().view || (isArchiveView(earlyView) ? earlyView : null) || storedArchiveView() || "cards";
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

    if (updateHash) syncHash();
  }

  if (viewButtons.length && viewPanels.length) {
    viewButtons.forEach((button) => {
      button.addEventListener("click", () => {
        setArchiveView(button.dataset.archiveView);
      });
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

  // 分类/标签的筛选按钮值来自跨文章聚合后的 term（大小写已按 hugo_title_case
  // 规整），而每篇文章卡片上的 data-tags/data-categories 是 frontmatter 原始大
  // 小写；同一个标签在不同文章里大小写不一致时（如 "Vue" / "vue"）会被聚合成
  // 一个筛选按钮，因此这里按大小写不敏感比较，避免筛选结果比聚合计数少。
  function hasValue(list, value) {
    const target = value.toLowerCase();
    return list.some((v) => v.toLowerCase() === target);
  }

  function applyFilter(type, value, options = {}) {
    const { updateHash = false } = options;
    let visibleCount = 0;

    items.forEach((item) => {
      const visible = type === "all" || hasValue(valuesFor(item, type), value);
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

    activeFilter = { type, value };
    if (updateHash) syncHash();
  }

  // hash 里的筛选值优先对回一个真实按钮（大小写不敏感），让选中态跟着亮；
  // 对不上就按原值直接筛，功能不受影响。
  function applyFilterFromHash(filter) {
    if (!filter) {
      applyFilter("all", "");
      return;
    }
    const target = filter.value.toLowerCase();
    const button = buttons.find(
      (b) => b.dataset.filterType === filter.type && (b.dataset.filterValue || "").toLowerCase() === target
    );
    if (button) applyFilter(button.dataset.filterType, button.dataset.filterValue);
    else applyFilter(filter.type, filter.value);
  }

  buttons.forEach((button) => {
    button.addEventListener("click", () => {
      applyFilter(button.dataset.filterType, button.dataset.filterValue, { updateHash: true });
    });
  });

  window.addEventListener("hashchange", () => {
    const { view, filter } = parseHash();
    if (view) setArchiveView(view, { updateHash: false });
    applyFilterFromHash(filter);
  });

  const initialFilter = parseHash().filter;
  if (initialFilter) {
    applyFilterFromHash(initialFilter);
  } else {
    const initial = buttons.find((button) => button.classList.contains("active")) || buttons[0];
    applyFilter(initial.dataset.filterType, initial.dataset.filterValue);
  }
});
