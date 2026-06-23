const toc = document.querySelector("[data-article-toc-desktop]");
const headings = [...document.querySelectorAll(".post-article .prose h2[id], .post-article .prose h3[id]")];

if (toc && headings.length) {
  const links = [...toc.querySelectorAll('a[href^="#"]')];
  let ticking = false;

  const updateActiveLink = () => {
    let activeId = headings[0].id;
    const threshold = window.innerHeight * .3;

    for (const heading of headings) {
      if (heading.getBoundingClientRect().top <= threshold) activeId = heading.id;
      else break;
    }

    for (const link of links) {
      let targetId = link.hash.slice(1);
      try { targetId = decodeURIComponent(targetId); } catch {}
      const isActive = targetId === activeId;
      link.classList.toggle("active", isActive);
      if (isActive) link.setAttribute("aria-current", "location");
      else link.removeAttribute("aria-current");
    }
    ticking = false;
  };

  window.addEventListener("scroll", () => {
    if (ticking) return;
    ticking = true;
    window.requestAnimationFrame(updateActiveLink);
  }, { passive: true });
  updateActiveLink();
}
