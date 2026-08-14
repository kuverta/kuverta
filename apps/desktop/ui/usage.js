// What the models cost, in tokens: today, this month, since counting began,
// by model and by job, and the last thirty days as bars. Only counts are
// kept — never what was asked or answered.

const usagePage = SETTINGS_PAGES.usage;
const tokenCount = new Intl.NumberFormat(undefined);
const tokenShort = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });

const USAGE_JOBS = {
  assistant: "The assistant",
  chat: "Sorting mail",
  vision: "Reading scans",
  trial: "Trying models",
};

function usageLayout() {
  usagePage.innerHTML = `
    <p class="lead">${t("How many tokens the models were sent and wrote back, counted on this computer for every job: the assistant, sorting mail, judging urgency, tasks and reading scans. A hosted service bills by these; a model on this computer costs time and power instead.")}</p>
    <div class="usage-tiles" data-el="tiles"></div>
    <section class="card">
      <h3>${t("The last 30 days")} <span class="hint">${t("tokens in and out per day")}</span></h3>
      <div class="usage-bars" data-el="bars"></div>
    </section>
    <section class="card">
      <h3>${t("This month by model")}</h3>
      <div class="item-list" data-list="models" data-empty="${t("Nothing counted this month.")}"></div>
    </section>
    <section class="card">
      <h3>${t("This month by job")}</h3>
      <div class="item-list" data-list="jobs" data-empty="${t("Nothing counted this month.")}"></div>
    </section>
    <section class="card">
      <p class="hint" data-el="since"></p>
      <div class="card-actions">
        <button type="button" data-el="refresh">${t("Refresh")}</button>
        <span class="spacer"></span>
        <button type="button" data-el="clear" class="danger">${t("Reset the statistics")}</button>
      </div>
    </section>
  `;
  usagePage.querySelector("[data-el=refresh]").onclick = fillUsage;
  usagePage.querySelector("[data-el=clear]").onclick = async () => {
    if (!confirm(t("Reset the token statistics? Counting starts again from nothing."))) return;
    try {
      await invoke("clear_usage_stats");
      await fillUsage();
    } catch (err) {
      say(String(err), true);
    }
  };
}

function usageTile(title, totals) {
  const tile = document.createElement("section");
  tile.className = "card usage-tile";
  const head = document.createElement("h3");
  head.textContent = title;
  const big = document.createElement("div");
  big.className = "usage-big";
  big.textContent = tokenShort.format(totals.input_tokens + totals.output_tokens);
  big.title = tokenCount.format(totals.input_tokens + totals.output_tokens);
  const sub = document.createElement("div");
  sub.className = "hint";
  sub.textContent = t("{input} in · {output} out · {calls} requests", {
    input: tokenCount.format(totals.input_tokens),
    output: tokenCount.format(totals.output_tokens),
    calls: tokenCount.format(totals.calls),
  });
  tile.append(head, big, sub);
  return tile;
}

function usageItem(title, sub, totals) {
  const item = document.createElement("div");
  item.className = "item";
  const text = document.createElement("div");
  text.className = "grow";
  const name = document.createElement("div");
  name.className = "title";
  name.textContent = title;
  text.append(name);
  if (sub) {
    const line = document.createElement("div");
    line.className = "sub";
    line.textContent = sub;
    text.append(line);
  }
  const numbers = document.createElement("div");
  numbers.className = "usage-numbers";
  numbers.textContent = t("{input} in · {output} out · {calls} requests", {
    input: tokenShort.format(totals.input_tokens),
    output: tokenShort.format(totals.output_tokens),
    calls: tokenCount.format(totals.calls),
  });
  item.append(text, numbers);
  return item;
}

function usageBars(days) {
  const box = usagePage.querySelector("[data-el=bars]");
  box.textContent = "";
  const most = Math.max(1, ...days.map((day) => day.input_tokens + day.output_tokens));
  const dayName = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short" });
  for (const day of days) {
    const total = day.input_tokens + day.output_tokens;
    const column = document.createElement("div");
    column.className = "usage-bar";
    column.title = `${dayName.format(new Date(`${day.day}T12:00:00`))}: ${t("{input} in · {output} out · {calls} requests", {
      input: tokenCount.format(day.input_tokens),
      output: tokenCount.format(day.output_tokens),
      calls: tokenCount.format(day.calls),
    })}`;
    const output = document.createElement("div");
    output.className = "out";
    output.style.height = `${(day.output_tokens / most) * 100}%`;
    const input = document.createElement("div");
    input.className = "in";
    input.style.height = `${(day.input_tokens / most) * 100}%`;
    column.append(output, input);
    if (!total) column.classList.add("empty");
    box.append(column);
  }
}

async function fillUsage() {
  if (!usagePage.firstElementChild) usageLayout();
  let stats;
  try {
    stats = await invoke("usage_stats");
  } catch (err) {
    say(String(err), true);
    return;
  }
  const tiles = usagePage.querySelector("[data-el=tiles]");
  tiles.textContent = "";
  tiles.append(
    usageTile(t("Today"), stats.today),
    usageTile(t("This month"), stats.this_month),
    usageTile(t("Since counting began"), stats.all_time),
  );
  usageBars(stats.days);

  const models = usagePage.querySelector("[data-list=models]");
  models.textContent = "";
  for (const share of stats.by_model) {
    const where = share.local ? t("{provider}, on this computer", { provider: share.provider }) : share.provider;
    models.append(usageItem(share.name, where, share));
  }
  const jobs = usagePage.querySelector("[data-list=jobs]");
  jobs.textContent = "";
  for (const share of stats.by_task) {
    jobs.append(usageItem(t(USAGE_JOBS[share.name] ?? share.name), "", share));
  }
  usagePage.querySelector("[data-el=since]").textContent = stats.since
    ? t("Counted since {date}.", { date: dateOnly.format(new Date(`${stats.since}T12:00:00`)) })
    : t("Nothing has been counted yet: counting starts with the next question to a model.");
}

usagePage.addEventListener("show", fillUsage);
