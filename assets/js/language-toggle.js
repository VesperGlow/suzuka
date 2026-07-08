(() => {
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

  // 不拦截导航：跳转立即发生，图标形变动画只在浏览器加载下一页的间隙里
  // 播放（此前 preventDefault + 220ms 延时跳转，等于给每次切换加人为等待）。
  document.querySelectorAll("[data-language-toggle]").forEach((toggle) => {
    toggle.addEventListener("click", (event) => {
      if (
        event.defaultPrevented ||
        event.button !== 0 ||
        event.metaKey ||
        event.ctrlKey ||
        event.shiftKey ||
        event.altKey ||
        reducedMotion.matches
      ) {
        return;
      }

      const switchingToEnglish = toggle.classList.contains("is-zh");
      toggle.classList.toggle("is-zh", !switchingToEnglish);
      toggle.classList.toggle("is-en", switchingToEnglish);
    });
  });
})();
