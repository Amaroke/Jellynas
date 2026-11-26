const DAYS = 30;
const FADE = { nyaa: "99", other: "55" };
const colorOf = item => (ORIGINS[item.origin] || ORIGINS.unknown) + (FADE[item.tracker] ?? "");
const nameOf = item => item.tracker ? `${item.origin} · ${item.tracker}` : item.origin;
const ORIGINS = {
  radarr: "#fbbf24", sonarr: "#38bdf8", lazylibrarian: "#a78bfa", farm: "#34d399", "cross-seed": "#f472b6", unknown: "#94a3b8",
};
const TIERS = ["#fb7185", "#fbbf24", "#facc15", "#34d399", "#a78bfa"];
const BEHAVIOURS = ["free", "clean_public", "hold", "adopt"];
const NUMBERS = {
  farm_size_gb: { step: "1", min: 1, max: 100000 },
  floor: { step: "0.05", min: 0.5, max: 5 },
  free_max_gb: { step: "0.5", min: 0.5, max: 1000 },
};
const WORDS = {
  tierNames: { blocked: "blocked", even: "even", floor: "floor", goal: "goal" },
  switches: {
    free: "Freeleech", clean_public: "Clean Public torrents",
    hold: "Hold Prowlarr under the floor", adopt: "Adopt the Library",
  },
  numbers: { farm_size_gb: "Farm budget (GB)", floor: "Ratio floor", free_max_gb: "Freeleech max size (GB)" },
  explainSwitches: {
    free: "Grabs the freeleech releases of C411 up to the freeleech max size, the most wanted first. Their download is not counted, their upload is.",
    clean_public: "Removes a public torrent after 7 days of seeding or once it uploaded its own size, whether a C411 copy exists or not. Its files stay while a C411 copy seeds them.",
    hold: "Under the floor, pauses C411 in Prowlarr so Radarr and Sonarr stop downloading there. Everything keeps seeding.",
    adopt: "Moves a C411 torrent of the Library into the Farm 6 hours after it finished, so it counts in the Farm budget.",
  },
  explainNumbers: {
    farm_size_gb: "Disk the Farm may fill. Once full, nothing more is grabbed and nothing is removed.",
    floor: "Account ratio under which no paid download happens on C411, not even for the Library, and Prowlarr holds C411 while Hold is on. Freeleech goes on.",
    free_max_gb: "Largest freeleech release the Farm grabs.",
  },
  explainOrigins: {
    radarr: "Movie of the Library", sonarr: "Show of the Library", lazylibrarian: "Book of the Library",
    farm: "Grabbed or moved into the Farm", "cross-seed": "Added by cross-seed", unknown: "Origin not recorded",
  },
};
const MERGE = {
  CLEAN: ["ready to merge", "ok"],
  HAS_HOOKS: ["ready to merge", "ok"],
  UNSTABLE: ["ready, optional checks pending", "ok"],
  BEHIND: ["behind the base", "wait"],
  BLOCKED: ["blocked by the rules", "wait"],
  DIRTY: ["conflicts", "bad"],
  DRAFT: ["draft", "muted"],
};
const REVIEW = { APPROVED: "approved", CHANGES_REQUESTED: "changes requested", REVIEW_REQUIRED: "review required" };
const CI = { SUCCESS: ["green", "ok"], FAILURE: ["failing", "bad"], ERROR: ["failing", "bad"], PENDING: ["running", "wait"], EXPECTED: ["waiting", "wait"] };

const unsaved = new Map();

function number(value, digits) {
  if (value === null || value === undefined) return "-";
  return Number(value).toLocaleString("en-GB", { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

function size(value, digits = 1) {
  return `${number(value, digits)} GB`;
}

function tag(tagName, attributes, ...children) {
  return element(tagName, attributes, ...children.filter(child => child !== null && child !== undefined && child !== ""));
}

function note(panel) {
  if (!panel || !panel.error) return null;
  return panel.reading
    ? tag("p", { class: "note" }, `Stale since ${clock(panel.updated)}: ${panel.error}`)
    : tag("p", { class: "note failed" }, `No data: ${panel.error}`);
}

async function send(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "X-Dashboard": "1", ...(body ? { "Content-Type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  }).catch(() => null);
  return Boolean(response && response.ok);
}

function farmControl(button, name, value) {
  button.disabled = true;
  send("/farm/controls", { name, value }).then(ok => {
    if (ok) unsaved.delete(name);
    else unsaved.set(name, `${WORDS.switches[name] || WORDS.numbers[name] || name} was not saved`);
    refresh();
  });
}

function toggle(name, on) {
  const button = tag("button", { type: "button", class: `pill${on ? " on" : ""}`, "aria-pressed": String(on) }, on ? "on" : "off");
  button.onclick = () => farmControl(button, name, !on);
  return button;
}

function kpi(css, label, value, sub) {
  return tag("div", { class: `kpi ${css}` }, tag("span", { class: "label" }, label), tag("span", { class: "value" }, value), tag("span", { class: "sub" }, sub || ""));
}

function kpis(board) {
  const account = board.account || {};
  const today = (board.history || []).at(-1) || {};
  const day = value => value === undefined || value === null ? "" : `${size(value)} in 24 h`;
  return tag("div", { class: "kpis" },
    kpi("ratio", "Account ratio", number(account.ratio, 2)),
    kpi("up", "Uploaded", size(account.uploaded_gb), day(today.uploaded_day_gb)),
    kpi("down", "Downloaded", size(account.downloaded_gb), day(today.downloaded_day_gb)),
    kpi("zen", "Zen", number(account.zen, 0)),
    kpi("seeds", "Seeds on C411", number(board.seeds, 0)),
    kpi("available", "Effective GB", size(account.available_gb, 0)));
}

function gauge(board) {
  const tiers = board.tiers || [];
  const ratio = (board.account || {}).ratio;
  if (!tiers.length) return null;
  const low = Math.min(0.6, (ratio ?? 1) * 0.9);
  const top = Math.max(2.2, tiers.at(-1).ratio * 1.1, (ratio ?? 0) * 1.05);
  const at = value => Math.min(Math.max(100 * (value - low) / (top - low), 0), 100);
  const edges = [low, ...tiers.map(tier => tier.ratio), top];
  const bands = edges.slice(1).map((high, index) =>
    tag("span", { style: `width:${at(high) - at(edges[index])}%;background:${TIERS[index % TIERS.length]}` }));
  const ticks = tiers.map(tier => tag("span", { class: "tick", style: `left:${at(tier.ratio)}%` }, number(tier.ratio, 1)));
  const cursor = ratio === undefined || ratio === null ? null : tag("span", { class: "cursor", style: `left:${at(ratio)}%` }, number(ratio, 2));
  const rows = tiers.map((tier, index) => {
    const gap = tier.gap_gb;
    const text = gap === null || gap === undefined ? "" : gap > 0 ? `${size(Math.abs(gap))} to upload` : `${size(Math.abs(gap))} of margin`;
    return tag("li", { class: gap > 0 ? "ahead" : "behind" },
      tag("span", { class: "swatch", style: `background:${TIERS[(index + 1) % TIERS.length]}` }),
      tag("span", { class: "name" }, `${number(tier.ratio, 1)} ${WORDS.tierNames[tier.name] || tier.name}`),
      tag("span", { class: "amount" }, text));
  }).reverse();
  return tag("figure", { class: "panel" }, tag("figcaption", {}, "Ratio tiers"),
    tag("div", { class: "track" }, cursor, tag("div", { class: "gauge" }, ...bands), ...ticks),
    tag("ul", { class: "tierlist" }, ...rows));
}

function windowOf(history) {
  const end = Date.parse(`${history.at(-1).day}T00:00:00Z`) / 1000;
  const start = end - (DAYS - 1) * 86400;
  return history.filter(entry => Date.parse(`${entry.day}T00:00:00Z`) / 1000 >= start);
}

function chartBox(caption) {
  const box = tag("div", { class: "plot" });
  return [tag("figure", { class: "panel chart" }, tag("figcaption", {}, caption), box), box];
}

function color(name) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function trends(board, draws) {
  const entries = windowOf(board.history);
  const days = entries.map(entry => Date.parse(`${entry.day}T00:00:00Z`) / 1000);
  const [zen, zenBox] = chartBox("Zen");
  const [transfer, transferBox] = chartBox("Upload and download");
  draws.push(keep => {
    if (typeof uPlot === "undefined") {
      zenBox.textContent = transferBox.textContent = "Charts need uPlot, which did not load.";
      return;
    }
    const muted = color("--muted");
    const line = color("--line");
    const day = (u, values) => values.map(value => new Date(value * 1000).toLocaleDateString("en-GB", { day: "2-digit", month: "2-digit" }));
    const axis = extra => ({ stroke: muted, grid: { stroke: line, width: 1 }, ticks: { stroke: line, width: 1 }, font: "11px system-ui", ...extra });
    const make = (box, options, data) => {
      const plot = new uPlot({ width: box.clientWidth || 300, height: 200, ...options }, data, box);
      const observer = new ResizeObserver(() => plot.setSize({ width: box.clientWidth, height: 200 }));
      observer.observe(box);
      keep.push({ plot, observer });
    };
    const shown = (unit, digits) => (u, value) => value === null || value === undefined ? "-" : `${number(value, digits)}${unit}`;
    make(zenBox, {
      scales: { x: { time: false }, paid: {}, zen: {} },
      axes: [axis({ values: day }), axis({ scale: "paid", size: 44 }), axis({ scale: "zen", side: 1, grid: { show: false }, size: 52 })],
      series: [
        { value: (u, value) => value === null ? "" : day(u, [value])[0] },
        { label: "Paid each night", scale: "paid", stroke: "#fbbf24", fill: "#fbbf24aa", paths: uPlot.paths.bars({ size: [0.6, 14] }), points: { show: false }, value: shown("", 0) },
        { label: "Balance", scale: "zen", stroke: "#34d399", width: 2, fill: "#34d39933", points: { show: false }, value: shown("", 0) },
      ],
    }, [days, entries.map(entry => entry.zen_paid ?? null), entries.map(entry => entry.zen ?? null)]);
    make(transferBox, {
      scales: { x: { time: false } },
      axes: [axis({ values: day }), axis({ size: 64, values: (u, values) => values.map(value => `${value} GB`) })],
      series: [
        { value: (u, value) => value === null ? "" : day(u, [value])[0] },
        { label: "Uploaded", stroke: "#60a5fa", width: 2, fill: "#60a5fa33", points: { show: false }, value: shown(" GB", 1) },
        { label: "Downloaded", stroke: "#fb7185", width: 2, fill: "#fb718522", points: { show: false }, value: shown(" GB", 1) },
      ],
    }, [days, entries.map(entry => entry.uploaded_gb ?? null), entries.map(entry => entry.downloaded_gb ?? null)]);
  });
  return tag("div", { class: "charts" }, zen, transfer);
}

function setting(title, hint, control) {
  return tag("div", { class: "setting" }, tag("div", {}, tag("div", { class: "title" }, title), tag("div", { class: "hint" }, hint)), control);
}

function numberForm(name, value) {
  const limits = NUMBERS[name];
  const input = tag("input", { type: "number", step: limits.step, min: String(limits.min), max: String(limits.max), value: String(value), "aria-label": WORDS.numbers[name] });
  const save = tag("button", { type: "submit", class: "save" }, "Save");
  const form = tag("form", { class: "number" }, input, save);
  form.onsubmit = event => {
    event.preventDefault();
    const parsed = Number(input.value);
    if (!Number.isFinite(parsed) || parsed < limits.min || parsed > limits.max) {
      input.setCustomValidity(`Between ${limits.min} and ${limits.max}`);
      input.reportValidity();
      return;
    }
    farmControl(save, name, parsed);
  };
  return form;
}

function settings(board) {
  const controls = board.controls || {};
  const switches = controls.switches || {};
  const toggles = BEHAVIOURS.filter(name => name in switches).map(name =>
    setting(WORDS.switches[name], WORDS.explainSwitches[name], toggle(name, switches[name])));
  const numbers = Object.entries(controls.numbers || {}).filter(([name]) => name in NUMBERS).map(([name, value]) =>
    setting(WORDS.numbers[name], WORDS.explainNumbers[name], numberForm(name, value)));
  return tag("div", { class: "pair" },
    tag("figure", { class: "panel" }, tag("figcaption", {}, "Farm switches"), ...toggles),
    tag("figure", { class: "panel" }, tag("figcaption", {}, "Farm settings"), ...numbers));
}

function donut(farm) {
  const budget = farm.budget_gb || 1;
  const used = farm.slices.reduce((sum, slice) => sum + slice.gb, 0);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "-1 -1 2 2");
  const circle = (stroke, part, offset) => {
    const node = document.createElementNS("http://www.w3.org/2000/svg", "circle");
    for (const [name, value] of Object.entries({ r: "0.84", pathLength: "100", stroke, "stroke-dasharray": `${part} ${100 - part}`, "stroke-dashoffset": String(-offset) })) node.setAttribute(name, value);
    return node;
  };
  svg.append(circle("var(--line)", 100, 0));
  const legend = [];
  let start = 0;
  for (const slice of farm.slices) {
    const part = 100 * slice.gb / Math.max(budget, used);
    const swatch = colorOf(slice);
    svg.append(circle(swatch, part, start));
    legend.push(tag("li", {}, tag("span", { class: "swatch", style: `background:${swatch}` }), tag("span", { class: "name" }, nameOf(slice)),
      tag("span", { class: "amount" }, size(slice.gb)), tag("span", { class: "meta" }, WORDS.explainOrigins[slice.origin] || "")));
    start += part;
  }
  legend.push(tag("li", {}, tag("span", { class: "swatch", style: "background:var(--line)" }), tag("span", { class: "name" }, "Free"),
    tag("span", { class: "amount" }, size(Math.max(budget - used, 0)))));
  return tag("figure", { class: "panel" }, tag("figcaption", {}, "Farm disk"),
    tag("div", { class: "disk" },
      tag("div", { class: "donut" }, svg, tag("div", { class: "center" }, tag("b", {}, `${number(100 * used / budget, 0)} %`), `of ${number(budget, 0)} GB`)),
      tag("ul", { class: "legend" }, ...legend)));
}

function ratioOf(up, down) {
  return down ? Math.round(100 * up / down) / 100 : null;
}

function earned(origins) {
  const rows = (name, swatch, origin) => [
    tag("tr", {}, tag("td", {}, swatch, name), ...[String(origin.torrents), size(origin.downloaded_gb), size(origin.uploaded_gb), number(origin.ratio, 2)].map(cell => tag("td", {}, cell))),
  ];
  const body = origins.flatMap(origin => rows(nameOf(origin), tag("span", { class: "swatch", style: `background:${colorOf(origin)}` }), origin));
  const keys = ["torrents", "downloaded_gb", "uploaded_gb"];
  const total = Object.fromEntries(keys.map(key => [key, origins.reduce((sum, origin) => sum + origin[key], 0)]));
  total.ratio = ratioOf(total.uploaded_gb, total.downloaded_gb);
  const foot = origins.length ? rows("Total", null, total) : [];
  return tag("figure", { class: "panel earned" }, tag("figcaption", {}, "Earned by origin"),
    tag("div", { class: "scroll" }, tag("table", {},
      tag("thead", {}, tag("tr", {}, ...["Origin", "Torrents", "Down", "Up", "Ratio"].map(cell => tag("th", {}, cell)))),
      tag("tbody", {}, ...body), tag("tfoot", {}, ...foot))));
}

function farmBlock(panel, draws) {
  const board = panel && panel.reading;
  if (!board) return tag("div", { class: "native" }, note(panel) || tag("p", { class: "empty" }, "Waiting for the Farm board"));
  if (!(board.history || []).length) {
    return tag("div", { class: "native" }, note(panel), tag("p", { class: "empty" }, "No history yet, ratio records its first day after its first run."));
  }
  const failures = [...unsaved.values()].map(text => tag("p", { class: "note failed" }, text));
  return tag("div", { class: "native farm" }, note(panel), ...failures, kpis(board), gauge(board), trends(board, draws), settings(board),
    tag("div", { class: "pair" }, donut(board.farm), earned(board.origins || [])));
}

function age(created) {
  const seconds = (Date.now() - Date.parse(created)) / 1000;
  if (!Number.isFinite(seconds)) return "";
  for (const [unit, length] of [["d", 86400], ["h", 3600], ["min", 60]]) {
    if (seconds >= length) return `${Math.floor(seconds / length)} ${unit}`;
  }
  return "just now";
}

function pullsBlock(panel) {
  const pulls = ((panel && panel.reading) || {}).pulls || [];
  const tones = pulls.map(pull => pull.tone);
  const count = name => tones.filter(tone => tone === name).length;
  const head = tag("div", { class: "summary" }, tag("b", {}, `${pulls.length} open`),
    tag("span", {}, `${count("ok")} ready · ${count("wait")} waiting · ${count("bad")} to fix`));
  const rows = pulls.map(pull => {
    const ci = CI[pull.ci] || ["none", ""];
    const chips = [tag("span", { class: "chip" }, pull.repo), tag("span", { class: "chip" }, pull.author), tag("span", { class: `chip ${ci[1]}` }, `CI ${ci[0]}`)];
    if (MERGE[pull.merge]) chips.push(tag("span", { class: `chip ${MERGE[pull.merge][1]}` }, MERGE[pull.merge][0]));
    if (REVIEW[pull.review]) chips.push(tag("span", { class: "chip" }, REVIEW[pull.review]));
    for (const label of pull.labels || []) chips.push(tag("span", { class: "chip" }, label));
    chips.push(tag("span", { class: "size" }, tag("span", { class: "add" }, `+${pull.additions || 0}`), " ", tag("span", { class: "del" }, `−${pull.deletions || 0}`), ` · ${pull.files || 0} file(s)`));
    const when = age(pull.created);
    if (when) chips.push(tag("span", {}, when));
    const link = (text, suffix) => tag("a", { href: `${pull.url}${suffix}`, target: "_blank", rel: "noopener" }, text);
    return tag("article", { class: `pull ${pull.tone}` },
      tag("div", { class: "what" },
        tag("a", { class: "title", href: pull.url, target: "_blank", rel: "noopener" }, tag("span", {}, `#${pull.number}`), ` ${pull.title}`),
        tag("div", { class: "meta" }, ...chips)),
      tag("nav", {}, link("Files", "/files"), link("Checks", "/checks"), link("Merge", "#partial-pull-merging")));
  });
  const empty = tag("p", { class: "empty" }, "No open pull request");
  return tag("div", { class: "native pulls" }, note(panel), head, ...(rows.length ? rows : [empty]));
}

function native(slug) {
  const draws = [];
  let node = null;
  if (slug === "farm") node = farmBlock(board.farm, draws);
  if (slug === "pull-requests") node = pullsBlock(board.pulls);
  if (node) node.draws = draws;
  return node;
}

function mount(block) {
  block.charts = [];
  for (const draw of block.draws || []) draw(block.charts);
}

function unmount(block) {
  for (const chart of block.charts || []) {
    chart.observer.disconnect();
    chart.plot.destroy();
  }
  block.charts = [];
}
