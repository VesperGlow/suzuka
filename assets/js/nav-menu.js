(() => {
  const toggle = document.querySelector("[data-sidebar-toggle]");
  const sidebar = document.querySelector("[data-sidebar-nav]");
  if (!toggle || !sidebar) return;

  const root = document.documentElement;
  const collapseToggle = document.querySelector("[data-sidebar-collapse]");
  const desktopQuery = window.matchMedia("(min-width: 1025px)");

  const isOpen = () => sidebar.classList.contains("is-open");

  // 移动端抽屉打开时，用 inert 关掉背后被遮住的区域（正文、页脚、回到
  // 顶部按钮）——header 本身在抽屉之上，仍可操作；避免键盘 Tab 穿透到
  // 视觉上完全看不见的内容。
  const obscured = [
    document.getElementById("main-content"),
    document.querySelector(".site-footer"),
    document.querySelector("[data-back-to-top]"),
  ].filter(Boolean);

  const setOpen = (open) => {
    sidebar.classList.toggle("is-open", open);
    document.body.classList.toggle("sidebar-open", open);
    toggle.setAttribute("aria-expanded", String(open));
    const trapping = open && !desktopQuery.matches;
    for (const el of obscured) el.toggleAttribute("inert", trapping);
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

  // 收起/展开会改变正文栏宽，文字重排后同一滚动像素对应的内容就变了。
  // 切换前记住阅读线（视口上缘、header 之下）附近的块级元素，在 padding
  // 过渡动画期间逐帧把它钉回原来的视口位置，阅读进度不漂移。
  // 锚定收敛后仍可能剩 ≤0.5px 的滚动量化残差；再记住每次切换的
  // 起点→终点滚动映射，切回去时直接还原上次起点，残差不跨次累积。
  let scrollMemo = null;
  const keepReadingAnchor = () => {
    const main = document.getElementById("main-content");
    if (!main || window.scrollY < 40) {
      scrollMemo = null;
      return;
    }
    const readingLine = 64 + 24; // --header-height + 一点余量
    let anchor = null;
    for (const el of main.querySelectorAll("p, h1, h2, h3, h4, h5, h6, li, pre, blockquote, figure, table")) {
      const rect = el.getBoundingClientRect();
      if (rect.height > 0 && rect.bottom > readingLine) {
        anchor = el;
        break;
      }
    }
    if (!anchor) {
      scrollMemo = null;
      return;
    }
    const fromY = window.scrollY;
    // 上次切换后没滚动过，本次就是原路返回 → 终点精确取上次的起点
    const exactTarget = scrollMemo && Math.abs(fromY - scrollMemo.toY) < 1 ? scrollMemo.fromY : null;
    // 钉的不是锚点顶边，而是阅读线穿过锚点的比例位置：图片会随栏宽等比
    // 缩放、长段落会重新折行，只有比例点才对应用户正在看的那个内容点。
    // 锚点整体在阅读线之下时取 0，退化为钉顶边。
    const startRect = anchor.getBoundingClientRect();
    const ratio = Math.max(0, (readingLine - startRect.top) / startRect.height);
    const startPoint = startRect.top + ratio * startRect.height;
    const trackedPoint = () => {
      const rect = anchor.getBoundingClientRect();
      return rect.top + ratio * rect.height;
    };
    // 覆盖 .25s 过渡再留余量；reduced-motion 下无过渡，首帧即修正完毕
    const deadline = performance.now() + 400;
    const step = () => {
      const delta = trackedPoint() - startPoint;
      // 站点开了 html { scroll-behavior: smooth }，必须显式 instant，
      // 否则逐帧的平滑滚动互相叠加会跑飞
      if (delta) window.scrollBy({ top: delta, behavior: "instant" });
      if (performance.now() < deadline) {
        requestAnimationFrame(step);
        return;
      }
      // 过渡结束。锚点仍在原位说明没被用户滚动打断：落到精确终点并记录映射
      if (Math.abs(trackedPoint() - startPoint) < 2) {
        if (exactTarget !== null) window.scrollTo({ top: exactTarget, behavior: "instant" });
        scrollMemo = { fromY, toY: window.scrollY };
      } else {
        scrollMemo = null;
      }
    };
    requestAnimationFrame(step);
  };

  if (collapseToggle) {
    collapseToggle.addEventListener("click", () => {
      keepReadingAnchor();
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

  // 移动端抽屉左滑关闭：跟手拖动，松手按位移和速度决定收起还是弹回。
  // 竖向滚动不受影响（CSS 侧 touch-action: pan-y，这里也先判定手势方向再接管）。
  let swipeId = null;
  let swipeStartX = 0;
  let swipeStartY = 0;
  let swipeLastX = 0;
  let swipeLastT = 0;
  let swipeVelocity = 0;
  let swipeDragging = false;

  const settleDrawer = (open) => {
    // 先强制一次布局，让过渡从当前拖动位置继续，而不是跳回展开位再动画
    sidebar.getBoundingClientRect();
    sidebar.style.transition = "";
    sidebar.style.transform = "";
    setOpen(open);
  };

  sidebar.addEventListener(
    "touchstart",
    (event) => {
      if (desktopQuery.matches || !isOpen() || event.touches.length !== 1) return;
      const touch = event.touches[0];
      swipeId = touch.identifier;
      swipeStartX = swipeLastX = touch.clientX;
      swipeStartY = touch.clientY;
      swipeLastT = event.timeStamp;
      swipeVelocity = 0;
      swipeDragging = false;
    },
    { passive: true }
  );

  sidebar.addEventListener(
    "touchmove",
    (event) => {
      if (swipeId === null) return;
      const touch = Array.from(event.changedTouches).find((t) => t.identifier === swipeId);
      if (!touch) return;
      const dx = touch.clientX - swipeStartX;
      const dy = touch.clientY - swipeStartY;
      if (!swipeDragging) {
        // 向左划出一小段、且横向分量明显大于竖向，才认定是关闭手势
        if (dx > -12 || Math.abs(dx) < Math.abs(dy) * 1.2) return;
        swipeDragging = true;
        sidebar.style.transition = "none";
      }
      const dt = event.timeStamp - swipeLastT;
      if (dt > 0) swipeVelocity = (touch.clientX - swipeLastX) / dt;
      swipeLastX = touch.clientX;
      swipeLastT = event.timeStamp;
      sidebar.style.transform = `translateX(${Math.min(0, dx)}px)`;
    },
    { passive: true }
  );

  const endDrawerSwipe = () => {
    if (swipeId === null) return;
    const dx = swipeLastX - swipeStartX;
    const wasDragging = swipeDragging;
    swipeId = null;
    swipeDragging = false;
    if (!wasDragging) return;
    // 划过 35% 宽度，或松手瞬间还有明显向左的速度（px/ms），就收起
    const shouldClose = dx < -sidebar.offsetWidth * 0.35 || (swipeVelocity < -0.5 && dx < -20);
    settleDrawer(!shouldClose);
  };
  sidebar.addEventListener("touchend", endDrawerSwipe);
  sidebar.addEventListener("touchcancel", endDrawerSwipe);

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
