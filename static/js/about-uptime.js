const uptime = document.querySelector("[data-site-uptime]");

if (uptime) {
  const startedAt = Date.parse("2026-06-18T00:00:00+08:00");
  const second = 1000;
  const minute = 60 * second;
  const hour = 60 * minute;
  const day = 24 * hour;

  function updateUptime() {
    let elapsed = Math.max(0, Date.now() - startedAt);
    const days = Math.floor(elapsed / day);
    elapsed %= day;
    const hours = Math.floor(elapsed / hour);
    elapsed %= hour;
    const minutes = Math.floor(elapsed / minute);
    const seconds = Math.floor((elapsed % minute) / second);

    uptime.textContent = `凉花已常驻 ${days} 天 ${hours} 小时 ${minutes} 分钟 ${seconds} 秒`;
  }

  updateUptime();
  window.setInterval(updateUptime, second);
}
