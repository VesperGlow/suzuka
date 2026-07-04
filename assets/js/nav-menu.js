(() => {
  const toggle = document.querySelector("[data-sidebar-toggle]");
  const sidebar = document.querySelector("[data-sidebar-nav]");
  if (!toggle || !sidebar) return;

  const desktopQuery = window.matchMedia("(min-width: 1025px)");

  const isOpen = () => sidebar.classList.contains("is-open");

  const setOpen = (open) => {
    sidebar.classList.toggle("is-open", open);
    document.body.classList.toggle("sidebar-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  };

  toggle.addEventListener("click", (event) => {
    event.stopPropagation();
    setOpen(!isOpen());
  });

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

  // 切回桌面宽度时确保收起抽屉状态（桌面端侧栏常驻显示，不受 is-open 影响）。
  desktopQuery.addEventListener("change", (event) => {
    if (event.matches) setOpen(false);
  });
})();
