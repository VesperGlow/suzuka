// 标题画面 ADV 对话框的打字机效果；点击对话框可跳过，reduced-motion 直接显示全文。
(() => {
  const box = document.querySelector("[data-adv]");
  const text = document.querySelector("[data-typewriter]");
  if (!box || !text) return;

  const full = text.textContent;
  if (!full || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

  let shown = 0;
  let timer = 0;

  const finish = () => {
    clearTimeout(timer);
    text.textContent = full;
    box.classList.remove("is-typing");
    box.removeEventListener("click", finish);
  };

  const tick = () => {
    shown += 1;
    text.textContent = full.slice(0, shown);
    if (shown < full.length) timer = setTimeout(tick, 42);
    else finish();
  };

  box.classList.add("is-typing");
  box.addEventListener("click", finish);
  text.textContent = "";
  timer = setTimeout(tick, 400);
})();
