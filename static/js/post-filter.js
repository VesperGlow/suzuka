document.querySelectorAll("[data-post-filter-scope]").forEach((scope) => {
  const buttons = [...scope.querySelectorAll("[data-filter-type]")];
  const items = [...scope.querySelectorAll("[data-filter-item]")];
  const groups = [...scope.querySelectorAll("[data-filter-group]")];
  const empty = scope.querySelector("[data-filter-empty]");

  if (!buttons.length || !items.length) return;

  function valuesFor(item, type) {
    try {
      return JSON.parse(item.dataset[type] || "[]");
    } catch {
      return [];
    }
  }

  function applyFilter(type, value) {
    let visibleCount = 0;

    items.forEach((item) => {
      const visible = type === "all" || valuesFor(item, type).includes(value);
      item.hidden = !visible;
      item.classList.toggle("is-hidden", !visible);
      if (visible) visibleCount += 1;
    });

    groups.forEach((group) => {
      const visible = Boolean(group.querySelector("[data-filter-item]:not([hidden])"));
      group.hidden = !visible;
      group.classList.toggle("is-hidden", !visible);
    });

    if (empty) {
      const visible = visibleCount === 0;
      empty.hidden = !visible;
      empty.classList.toggle("is-hidden", !visible);
    }

    buttons.forEach((button) => {
      const active = button.dataset.filterType === type && button.dataset.filterValue === value;
      button.classList.toggle("active", active);
      button.setAttribute("aria-pressed", String(active));
    });
  }

  buttons.forEach((button) => {
    button.addEventListener("click", () => {
      applyFilter(button.dataset.filterType, button.dataset.filterValue);
    });
  });

  const initial = buttons.find((button) => button.classList.contains("active")) || buttons[0];
  applyFilter(initial.dataset.filterType, initial.dataset.filterValue);
});
