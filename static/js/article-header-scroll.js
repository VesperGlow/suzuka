(() => {
  const header = document.querySelector(".post-page .site-header");
  if (!header) return;

  const threshold = 10;
  let lastScrollY = window.scrollY;
  let direction = 0;
  let distance = 0;
  let ticking = false;

  const showHeader = () => {
    header.classList.remove("is-header-hidden");
    header.classList.add("is-header-visible");
    document.body.classList.remove("is-header-hidden");
    document.body.classList.add("is-header-visible");
  };

  const hideHeader = () => {
    header.classList.remove("is-header-visible");
    header.classList.add("is-header-hidden");
    document.body.classList.remove("is-header-visible");
    document.body.classList.add("is-header-hidden");
  };

  const updateHeader = () => {
    const scrollY = Math.max(window.scrollY, 0);
    const delta = scrollY - lastScrollY;
    const nextDirection = Math.sign(delta);

    if (scrollY <= 10) {
      showHeader();
      distance = 0;
    } else if (nextDirection !== 0) {
      if (nextDirection !== direction) distance = 0;
      direction = nextDirection;
      distance += Math.abs(delta);

      if (distance > threshold) {
        if (direction > 0 && scrollY > 120) hideHeader();
        if (direction < 0) showHeader();
        distance = 0;
      }
    }

    lastScrollY = scrollY;
    ticking = false;
  };

  window.addEventListener("scroll", () => {
    if (ticking) return;
    ticking = true;
    window.requestAnimationFrame(updateHeader);
  }, { passive: true });
})();
