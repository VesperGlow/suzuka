const backToTop = document.querySelector("[data-back-to-top]");

if (backToTop) {
  const tracksReading = backToTop.classList.contains("back-to-top-reading");

  const updateBackToTop = () => {
    backToTop.hidden = window.scrollY <= 600;
    if (!tracksReading) return;

    const scrollable = document.documentElement.scrollHeight - window.innerHeight;
    const progress = scrollable > 0 ? Math.min(Math.max(window.scrollY / scrollable, 0), 1) : 1;
    const visibleImageTop = 8.5;
    const visibleImageBottom = 92.7;
    const revealProgress = progress ** 2;
    const remainder = visibleImageBottom - revealProgress * (visibleImageBottom - visibleImageTop);
    backToTop.style.setProperty("--reading-remainder", `${remainder.toFixed(2)}%`);
  };

  window.addEventListener("scroll", updateBackToTop, { passive: true });
  backToTop.addEventListener("click", () => {
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    window.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
  });
  updateBackToTop();
}
