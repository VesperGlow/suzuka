(() => {
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");

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

      event.preventDefault();
      const target = toggle.href;
      const switchingToEnglish = toggle.classList.contains("is-zh");

      toggle.classList.toggle("is-zh", !switchingToEnglish);
      toggle.classList.toggle("is-en", switchingToEnglish);

      window.setTimeout(() => {
        window.location.assign(target);
      }, 220);
    });
  });
})();
