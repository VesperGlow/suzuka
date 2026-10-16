(() => {
  const toggle = document.querySelector("[data-nav-toggle]");
  const nav = document.querySelector("[data-site-nav]");
  if (!toggle || !nav) return;

  // 与 CSS 一致：窄屏展开横向菜单，大屏保持顶栏导航。
  const desktopQuery = window.matchMedia("(min-width: 1025px)");
  const isOpen = () => nav.classList.contains("is-open");
  let resizeTimer;

  const finishResize = () => {
    window.clearTimeout(resizeTimer);
    document.documentElement.classList.remove("nav-resizing");
  };
  const beginResize = () => {
    window.clearTimeout(resizeTimer);
    // 展开时暂停浏览器的滚动锚定，让正文跟随导航平滑移动。
    document.documentElement.classList.add("nav-resizing");
    const duration = parseFloat(getComputedStyle(nav).transitionDuration) * 1000;
    resizeTimer = window.setTimeout(finishResize, duration + 50);
  };

  const setOpen = (open, restoreFocus = false) => {
    const menuOpen = open && !desktopQuery.matches;
    if (desktopQuery.matches) finishResize();
    else if (menuOpen !== isOpen()) beginResize();
    nav.inert = !desktopQuery.matches && !menuOpen;
    nav.classList.toggle("is-open", menuOpen);
    toggle.setAttribute("aria-expanded", String(menuOpen));
    const label = menuOpen ? toggle.dataset.labelClose : toggle.dataset.labelOpen;
    toggle.setAttribute("aria-label", label);
    toggle.title = label;
    if (restoreFocus && !desktopQuery.matches) toggle.focus({ preventScroll: true });
  };

  toggle.addEventListener("click", (event) => {
    const open = !isOpen();
    setOpen(open);
    if (open && event.detail === 0) nav.querySelector("a")?.focus({ preventScroll: true });
  });
  toggle.addEventListener("keydown", (event) => {
    if (event.key !== "ArrowDown" || desktopQuery.matches) return;
    event.preventDefault();
    setOpen(true);
    nav.querySelector("a")?.focus({ preventScroll: true });
  });

  nav.addEventListener("click", (event) => {
    if (isOpen() && event.target.closest("a")) setOpen(false, true);
  });

  const isOutside = (target) => !nav.contains(target) && !toggle.contains(target);
  document.addEventListener("pointerdown", (event) => {
    if (isOpen() && isOutside(event.target)) setOpen(false);
  });
  document.addEventListener("focusin", (event) => {
    if (isOpen() && isOutside(event.target)) setOpen(false);
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && isOpen()) {
      event.preventDefault();
      setOpen(false, true);
    }
  });

  desktopQuery.addEventListener("change", () => {
    const focusInNav = nav.contains(document.activeElement);
    setOpen(false, !desktopQuery.matches && focusInNav);
  });
  nav.addEventListener("transitionend", (event) => {
    if (event.target === nav && event.propertyName === "grid-template-rows") finishResize();
  });

  setOpen(false);
  window.addEventListener("pagehide", finishResize);
})();
