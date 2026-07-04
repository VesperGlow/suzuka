const backToTop = document.querySelector("[data-back-to-top]");

if (backToTop) {
  const updateBackToTop = () => {
    backToTop.hidden = window.scrollY <= 600;
  };

  window.addEventListener("scroll", updateBackToTop, { passive: true });
  backToTop.addEventListener("click", () => {
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    window.scrollTo({ top: 0, behavior: reduceMotion ? "auto" : "smooth" });
  });
  updateBackToTop();
}
