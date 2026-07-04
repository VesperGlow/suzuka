(() => {
  const toggle = document.querySelector("[data-sidebar-toggle]");
  const sidebar = document.querySelector("[data-sidebar-nav]");
  if (!toggle || !sidebar) return;

  const root = document.documentElement;
  const collapseButton = document.querySelector("[data-sidebar-collapse]");
  const expandButton = document.querySelector("[data-sidebar-expand]");
  const desktopQuery = window.matchMedia("(min-width: 1025px)");

  const isOpen = () => sidebar.classList.contains("is-open");

  const setOpen = (open) => {
    sidebar.classList.toggle("is-open", open);
    document.body.classList.toggle("sidebar-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  };

  // 桌面端的收起状态，独立于移动端抽屉，跨页面记忆。
  const setCollapsed = (collapsed) => {
    root.classList.toggle("sidebar-collapsed", collapsed);
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

  if (collapseButton && expandButton) {
    collapseButton.addEventListener("click", () => {
      setCollapsed(true);
      expandButton.focus();
    });
    expandButton.addEventListener("click", () => {
      setCollapsed(false);
      collapseButton.focus();
    });
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
