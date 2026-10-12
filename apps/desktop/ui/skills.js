// The assistant on the web, and the skills it is taught: whether it may
// search and read pages when asked, where its searches go, and every skill
// — waiting to be read, or accepted — with the whole text in front of the
// person before the one button that accepts it.
//
// A skill is instructions from somebody else, so the rule here is the one a
// person applies to a program: read it before you run it. Accept lives at
// the bottom of the text, after whatever stood out in it; no tool of the
// assistant's can press it.

const researchPage = SETTINGS_PAGES.research;
const research = { settings: null, open: null };

function researchLayout() {
  researchPage.innerHTML = `
    <p class="lead">${t("Two things the assistant can do beyond your mail, each off until you switch it on here: look things up on the web when you ask it to, and follow skills — files of instructions for one kind of job — that you have read and accepted.")}</p>
    <section class="card">
      <h3>${t("Research online")}</h3>
      <label class="check"><input type="checkbox" data-el="research"> ${t("Let the assistant search the web and read pages when I ask it to")}</label>
      <p class="hint">${t("What it searches for goes to the search engine; the pages it reads come to this computer as text — no scripts, no pictures — and, with a hosted model, go on to that service. It may open only addresses you wrote, a search returned, or a page it read links to: never one from inside a message, so a link in a mail is opened only when you paste it. Once it has read from the web in a question, it can look and draft but not change mail, sign, or remember anything in that question — and nothing it read online can ever lead to a note, a task or a skill in that conversation.")}</p>
      <label>${t("Search engine")}<input data-el="search" placeholder="DuckDuckGo" autocomplete="off" spellcheck="false"></label>
      <p class="hint">${t("Left empty, DuckDuckGo. Or the address of a SearXNG of your own, such as http://192.168.8.5:8080 — then searches go there and nowhere else.")}</p>
    </section>
    <section class="card">
      <h3>${t("Skills")}</h3>
      <label class="check"><input type="checkbox" data-el="skills"> ${t("Use skills")}</label>
      <p class="hint">${t("A skill is a text file telling the assistant how to do one kind of job — how a cancellation is worded, what to check on a supplier's invoice. The assistant downloads one when you give it the address, or you add one here. Nothing in a skill takes effect until you have read it here and accepted it: it is instructions the assistant will follow, so read it as you would a program before installing it. Beside the text, words that stood out are listed — addresses, talk of hiding, ignoring rules or sending something somewhere. They are only words that stood out, not a verdict.")}</p>
      <form data-form="add" class="memory-add" autocomplete="off">
        <input name="url" placeholder="https://…/SKILL.md" spellcheck="false">
        <button type="submit">${t("Download")}</button>
      </form>
      <div class="item-list skills" data-list="skills" data-empty="${t("No skills yet.")}"></div>
    </section>
  `;
  for (const name of ["research", "skills"]) {
    researchPage.querySelector(`[data-el=${name}]`).addEventListener("change", (event) => {
      saveResearch({ [name]: event.target.checked });
    });
  }
  const search = researchPage.querySelector("[data-el=search]");
  search.addEventListener("change", () => saveResearch({ search: search.value }));
  search.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
      event.preventDefault();
      search.blur();
    }
  });
  researchPage.querySelector("[data-form=add]").addEventListener("submit", async (event) => {
    event.preventDefault();
    const form = event.target;
    const input = form.elements.url;
    const url = input.value.trim();
    if (!url) return;
    const button = form.querySelector("button");
    button.disabled = true;
    button.textContent = t("Downloading…");
    try {
      const skill = await invoke("add_skill", { url });
      input.value = "";
      research.open = skill.id;
      await fillSkills();
    } catch (err) {
      say(String(err), true);
    } finally {
      button.disabled = false;
      button.textContent = t("Download");
    }
  });
}

/// Saves one changed field with the rest as they stand; the page shows what
/// the core kept, which is the address trimmed.
async function saveResearch(change) {
  const settings = { ...research.settings, ...change };
  try {
    research.settings = await invoke("set_web_settings", { settings });
  } catch (err) {
    say(String(err), true);
  }
  showResearchSettings();
}

function showResearchSettings() {
  const settings = research.settings ?? { research: false, skills: false, search: "" };
  researchPage.querySelector("[data-el=research]").checked = settings.research;
  researchPage.querySelector("[data-el=skills]").checked = settings.skills;
  const search = researchPage.querySelector("[data-el=search]");
  if (document.activeElement !== search) search.value = settings.search;
}

function skillItem(skill) {
  const box = document.createElement("div");
  box.className = "skill";
  const row = document.createElement("div");
  row.className = "item";
  const words = document.createElement("div");
  words.className = "grow";
  const title = document.createElement("div");
  title.className = "title";
  title.textContent = skill.name;
  const sub = document.createElement("div");
  sub.className = "sub";
  const when = skill.accepted_at
    ? t("Accepted {when}", { when: new Date(skill.accepted_at * 1000).toLocaleDateString() })
    : t("Waiting for you to read it");
  sub.textContent = [skill.description, when, t("{count} characters", { count: skill.chars })]
    .filter(Boolean)
    .join(" · ");
  sub.title = skill.source;
  if (!skill.accepted_at) sub.classList.add("bad");
  words.append(title, sub);
  row.append(words);

  if (skill.accepted_at) {
    const enabled = document.createElement("label");
    enabled.className = "check";
    const box = document.createElement("input");
    box.type = "checkbox";
    box.checked = skill.enabled;
    box.onchange = async () => {
      try {
        await invoke("set_skill_enabled", { id: skill.id, enabled: box.checked });
      } catch (err) {
        box.checked = !box.checked;
        say(String(err), true);
      }
    };
    enabled.append(box, ` ${t("Enabled")}`);
    row.append(enabled);
  }

  const read = document.createElement("button");
  read.type = "button";
  read.textContent = research.open === skill.id ? t("Hide") : t("Read");
  read.onclick = async () => {
    research.open = research.open === skill.id ? null : skill.id;
    await fillSkills();
  };
  const remove = document.createElement("button");
  remove.type = "button";
  remove.className = "danger";
  remove.textContent = t("Remove");
  remove.onclick = async () => {
    try {
      await invoke("delete_skill", { id: skill.id });
      if (research.open === skill.id) research.open = null;
      await fillSkills();
    } catch (err) {
      say(String(err), true);
    }
  };
  row.append(read, remove);
  box.append(row);

  if (research.open === skill.id) {
    const reading = document.createElement("div");
    reading.className = "skill-reading";
    reading.textContent = t("Loading…");
    box.append(reading);
    invoke("skill_text", { id: skill.id })
      .then((full) => renderSkillReading(reading, skill, full))
      .catch((err) => {
        reading.textContent = String(err);
        reading.classList.add("bad");
      });
  }
  return box;
}

/// The text, what stood out in it above, and — for one still waiting — the
/// Accept button beneath, past the whole text.
function renderSkillReading(reading, skill, full) {
  reading.textContent = "";
  const from = document.createElement("p");
  from.className = "hint";
  from.textContent = t("From {source}", { source: full.source });
  reading.append(from);
  if (full.warnings.length) {
    const head = document.createElement("p");
    head.className = "hint bad";
    head.textContent = t("Worth a second look:");
    const list = document.createElement("ul");
    list.className = "skill-warnings";
    for (const warning of full.warnings) {
      const item = document.createElement("li");
      item.textContent = warning;
      list.append(item);
    }
    reading.append(head, list);
  } else {
    const none = document.createElement("p");
    none.className = "hint";
    none.textContent = t("Nothing stood out. Read it anyway: it is instructions the assistant will follow.");
    reading.append(none);
  }
  const text = document.createElement("pre");
  text.className = "skill-text";
  text.textContent = full.text;
  reading.append(text);
  if (!skill.accepted_at) {
    const actions = document.createElement("div");
    actions.className = "card-actions";
    const accept = document.createElement("button");
    accept.type = "button";
    accept.className = "primary";
    accept.textContent = t("Accept");
    accept.onclick = async () => {
      accept.disabled = true;
      try {
        await invoke("accept_skill", { id: skill.id });
        research.open = null;
        await fillSkills();
      } catch (err) {
        accept.disabled = false;
        say(String(err), true);
      }
    };
    actions.append(accept);
    reading.append(actions);
  }
}

async function fillSkills() {
  const list = researchPage.querySelector("[data-list=skills]");
  try {
    const skills = await invoke("skills");
    list.textContent = "";
    for (const skill of skills) list.append(skillItem(skill));
  } catch (err) {
    say(String(err), true);
  }
}

async function fillResearch() {
  if (!researchPage.firstElementChild) researchLayout();
  try {
    research.settings = await invoke("web_settings");
  } catch (err) {
    say(String(err), true);
  }
  showResearchSettings();
  await fillSkills();
}

researchPage.addEventListener("show", fillResearch);
