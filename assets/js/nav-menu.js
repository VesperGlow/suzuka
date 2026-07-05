(() => {
  const toggle = document.querySelector("[data-sidebar-toggle]");
  const sidebar = document.querySelector("[data-sidebar-nav]");
  if (!toggle || !sidebar) return;

  const root = document.documentElement;
  const collapseToggle = document.querySelector("[data-sidebar-collapse]");
  const desktopQuery = window.matchMedia("(min-width: 1025px)");

  const isOpen = () => sidebar.classList.contains("is-open");

  const setOpen = (open) => {
    sidebar.classList.toggle("is-open", open);
    document.body.classList.toggle("sidebar-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  };

  const syncCollapseToggle = () => {
    if (!collapseToggle) return;
    const collapsed = root.classList.contains("sidebar-collapsed");
    const label = collapsed ? collapseToggle.dataset.labelExpand : collapseToggle.dataset.labelCollapse;
    collapseToggle.setAttribute("aria-expanded", String(!collapsed));
    if (label) {
      collapseToggle.title = label;
      collapseToggle.setAttribute("aria-label", label);
    }
  };

  // 桌面端的收起状态，独立于移动端抽屉，跨页面记忆。
  // 刚收起时把手保持强调样式，一段时间不碰后淡化（悬停/聚焦时由 CSS 再次强化）。
  let handleFadeTimer;
  const setCollapsed = (collapsed) => {
    root.classList.toggle("sidebar-collapsed", collapsed);
    syncCollapseToggle();
    if (collapseToggle) {
      clearTimeout(handleFadeTimer);
      collapseToggle.classList.toggle("is-strong", collapsed);
      if (collapsed) {
        handleFadeTimer = setTimeout(() => collapseToggle.classList.remove("is-strong"), 2600);
      }
    }
    try {
      localStorage.setItem("sidebar-collapsed", collapsed ? "1" : "0");
    } catch {
      // 收起状态仍对当前页面生效，只是无法跨页面记住。
    }
  };

  toggle.addEventListener("click", (event) => {
    event.stopPropagation();
    setOpen(!isOpen());
  });

  if (collapseToggle) {
    collapseToggle.addEventListener("click", () => {
      setCollapsed(!root.classList.contains("sidebar-collapsed"));
    });
    // 页面加载时可能已由 <head> 内联脚本恢复了收起状态，这里同步按钮文案。
    syncCollapseToggle();
  }

  // 侧栏滚动条只在滚动时显现，停止滚动后自动淡出。
  for (const area of document.querySelectorAll(".sidebar-nav-inner, [data-sidebar-timeline]")) {
    let hideTimer;
    area.addEventListener(
      "scroll",
      () => {
        area.classList.add("is-scrolling");
        clearTimeout(hideTimer);
        hideTimer = setTimeout(() => area.classList.remove("is-scrolling"), 800);
      },
      { passive: true }
    );
  }

  // 点击侧栏内的链接后自动收起（移动端抽屉）。
  sidebar.addEventListener("click", (event) => {
    if (!desktopQuery.matches && event.target.closest("a")) setOpen(false);
  });

  document.addEventListener("click", (event) => {
    if (isOpen() && !sidebar.contains(event.target) && !toggle.contains(event.target)) {
      setOpen(false);
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && isOpen()) {
      setOpen(false);
      toggle.focus();
    }
  });

  // 切回桌面宽度时确保收起抽屉状态（桌面端侧栏由 sidebar-collapsed 控制，不受 is-open 影响）。
  desktopQuery.addEventListener("change", (event) => {
    if (event.matches) setOpen(false);
  });

  // 时间轴滚动到当前文章（手动计算，避免 scrollIntoView 连带滚动整个页面）。
  const timeline = document.querySelector("[data-sidebar-timeline]");
  const active = timeline && timeline.querySelector("a.active");
  if (timeline && active && timeline.scrollHeight > timeline.clientHeight) {
    const offset = active.getBoundingClientRect().top - timeline.getBoundingClientRect().top;
    timeline.scrollTop = Math.max(0, offset - (timeline.clientHeight - active.offsetHeight) / 2);
  }
})();
