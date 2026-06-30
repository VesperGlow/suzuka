(() => {
  const toggle = document.querySelector("[data-nav-menu-toggle]");
  const menu = document.querySelector("[data-nav-menu]");
  if (!toggle || !menu) return;

  const tools = toggle.closest("[data-header-tools]");
  const desktopQuery = window.matchMedia("(min-width: 769px)");

  const isOpen = () => menu.classList.contains("is-open");

  const setOpen = (open) => {
    menu.classList.toggle("is-open", open);
    toggle.setAttribute("aria-expanded", String(open));
    // 菜单与搜索互斥，避免两个浮层同时展开。
    if (open) tools?.classList.remove("search-active");
  };

  toggle.addEventListener("click", (event) => {
    event.stopPropagation();
    setOpen(!isOpen());
  });

  // 点击菜单内的链接后自动收起。
  menu.addEventListener("click", (event) => {
    if (event.target.closest("a.mobile-nav-link")) setOpen(false);
  });

  document.addEventListener("click", (event) => {
    if (isOpen() && !menu.contains(event.target) && !toggle.contains(event.target)) {
      setOpen(false);
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && isOpen()) {
      setOpen(false);
      toggle.focus();
    }
  });

  // 切回桌面宽度时确保收起。
  desktopQuery.addEventListener("change", (event) => {
    if (event.matches) setOpen(false);
  });
})();
