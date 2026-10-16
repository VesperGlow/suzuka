const backToTop = document.querySelector("[data-back-to-top]");

if (backToTop) {
  const updateBackToTop = () => {
    backToTop.hidden = window.scrollY <= 600;
    const scrollable = document.documentElement.scrollHeight - window.innerHeight;
    const progress = scrollable > 0 ? Math.min(Math.max(window.scrollY / scrollable, 0), 1) : 1;
    const percent = Math.round(progress * 100);
    backToTop.style.setProperty("--reading-progress", String(percent));
    const label = `${backToTop.dataset.label} · ${backToTop.dataset.progressLabel.replace("{count}", percent)}`;
    backToTop.setAttribute("aria-label", label);
    backToTop.title = label;
  };

  window.addEventListener("scroll", updateBackToTop, { passive: true });
  window.addEventListener("resize", updateBackToTop);
  backToTop.addEventListener("click", () => {
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    window.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
  });
  updateBackToTop();
}
