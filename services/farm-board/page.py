import base64
import datetime
import hashlib
import html
import itertools

HISTORY_DAYS = 30
PARTS = ("1", "2", "3")
COLORS = {
    "paid": "#f472b6", "free": "#34d399", "library": "#38bdf8",
    "quest": "#fb923c", "unknown": "#94a3b8", "public": "#64748b",
}
TIER_COLORS = ("#fb7185", "#fbbf24", "#facc15", "#34d399", "#a78bfa")
BEHAVIOURS = ("free", "twins", "paid", "clean_public", "drop_twins", "hold", "adopt")
NUMBER_STEPS = {"farm_size_gb": "1", "floor": "0.05", "daily_gb": "0.5", "max_size_gb": "0.01"}
NUMBER_LIMITS = {"farm_size_gb": (1, 100000), "floor": (0.5, 5), "daily_gb": (0, 1000), "max_size_gb": (0.01, 1000)}
TEXT = {
    "en": {
        "gb": "GB", "decimal": ".", "empty": "No history yet, ratio records its first day after its first run.",
        "ratio": "Account ratio", "floor": "floor {v}", "uploaded": "Uploaded", "credit": "{v} real, {c} bought",
        "downloaded": "Downloaded", "day": "{v} in 24 h", "zen": "Zen", "paid_last": "+{v} last night",
        "seeds": "Seeds on C411", "known": "{v} with a known category", "grade": "Class",
        "grades": {"bronze": "Bronze", "argent": "Silver", "or": "Gold"},
        "zen_chart": "Zen", "paid": "paid each night", "balance": "balance", "transfer": "Upload and download",
        "tiers": "Ratio tiers", "above": "Next tier {r}, {n}: {v} to upload", "below": "Tier under {r}, {n}: {v} of margin",
        "top": "Above every tier", "tier_names": {"blocked": "download blocked", "even": "even", "floor": "floor", "goal": "goal"},
        "challenges": "Quests", "voie": {"seed": "Seed path", "specialization": "Specialization path"},
        "voie_done": "{d} of {t} done here, {c} on C411",
        "names": {
            "quantity_seed": "Seed count", "duration_seed": "Seeding time", "vtr_min": "Seed note",
            "category_volume_seed": "Category volume", "category_diversity_seed": "Diversity",
        },
        "level": "level {l} of {n}", "finished": "every level done", "indicator": "indicator",
        "categories": "Category volume, a badge at 50 seeds", "on": "on", "off": "off",
        "behaviours": "Farm", "switches": {
            "free": "Freeleech", "twins": "Twins", "paid": "Fresh paid grabs", "clean_public": "Clean Public torrents",
            "drop_twins": "Drop Twins", "hold": "Hold Prowlarr under the floor", "adopt": "Adopt the Library",
        },
        "numbers": {"farm_size_gb": "Farm budget (GB)", "floor": "Ratio floor", "daily_gb": "Paid download per day (GB)", "max_size_gb": "Quest grab size (GB)"},
        "save": "Save",
        "disk": "Farm disk", "free": "Free", "of": "of {budget} {gb}",
        "earned": "Earned by Source", "columns": ("Source", "Torrents", "Down", "Up", "Ratio"), "total": "Total",
        "week": "7 days: {down} down, {up} up, ratio {ratio}, {days} days in the Farm",
        "explain": {
            "paid": "Fresh release bought with download", "free": "No download counted on C411",
            "library": "Movie or show of the Library", "quest": "Grabbed for a Quest",
            "public": "Public tracker copy, leaves within two weeks", "unknown": "Source not recorded",
        },
    },
    "fr": {
        "gb": "Go", "decimal": ",", "empty": "Pas encore d'historique, ratio note son premier jour après son premier passage.",
        "ratio": "Ratio du compte", "floor": "plancher {v}", "uploaded": "Envoyé", "credit": "{v} réels, {c} achetés",
        "downloaded": "Reçu", "day": "{v} en 24 h", "zen": "Zen", "paid_last": "+{v} cette nuit",
        "seeds": "Seeds sur C411", "known": "{v} de catégorie connue", "grade": "Classe",
        "grades": {"bronze": "Bronze", "argent": "Argent", "or": "Or"},
        "zen_chart": "Zen", "paid": "payé chaque nuit", "balance": "solde", "transfer": "Envoi et réception",
        "tiers": "Paliers de ratio", "above": "Palier suivant {r}, {n} : {v} à envoyer", "below": "Palier en dessous {r}, {n} : {v} de marge",
        "top": "Au-dessus de tous les paliers", "tier_names": {"blocked": "téléchargement bloqué", "even": "équilibre", "floor": "plancher", "goal": "objectif"},
        "challenges": "Quêtes", "voie": {"seed": "Voie seed", "specialization": "Voie spécialisation"},
        "voie_done": "{d} sur {t} faites ici, {c} sur C411",
        "names": {
            "quantity_seed": "Nombre de seeds", "duration_seed": "Durée de seed", "vtr_min": "Note des seeds",
            "category_volume_seed": "Volume catégorie", "category_diversity_seed": "Diversité",
        },
        "level": "palier {l} sur {n}", "finished": "tous les paliers faits", "indicator": "indicateur",
        "categories": "Volume catégorie, un badge à 50 seeds", "on": "actif", "off": "coupé",
        "behaviours": "Farm", "switches": {
            "free": "Freeleech", "twins": "Twins", "paid": "Grabs payants frais", "clean_public": "Nettoyage des Public torrents",
            "drop_twins": "Retrait des Twins", "hold": "Pause Prowlarr sous le plancher", "adopt": "Adoption de la Library",
        },
        "numbers": {"farm_size_gb": "Budget de la Farm (Go)", "floor": "Plancher de ratio", "daily_gb": "Download payant par jour (Go)", "max_size_gb": "Taille des grabs de quête (Go)"},
        "save": "Valider",
        "disk": "Disque de la Farm", "free": "Libre", "of": "sur {budget} {gb}",
        "earned": "Gains par Source", "columns": ("Source", "Torrents", "Reçu", "Envoyé", "Ratio"), "total": "Total",
        "week": "7 jours : {down} reçus, {up} envoyés, ratio {ratio}, {days} jours dans la Farm",
        "explain": {
            "paid": "Sortie fraîche payée en download", "free": "Aucun download compté sur C411",
            "library": "Film ou série de la Library", "quest": "Pris pour une Quête",
            "public": "Copie d'un tracker public, part sous deux semaines", "unknown": "Source non enregistrée",
        },
    },
}
STYLE = """
:root { --text: #e2e8f0; --muted: #94a3b8; --line: rgba(148, 163, 184, 0.22);
  --violet: #a78bfa; --green: #34d399; --amber: #fbbf24; --blue: #60a5fa; --rose: #fb7185; --cyan: #22d3ee; --orange: #fb923c; }
* { box-sizing: border-box; }
html, body { margin: 0; height: 100%; background: transparent; color: var(--text);
  font: 13px/1.35 Manrope, ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }
html.standalone { color-scheme: dark; background: radial-gradient(circle at 20% 0%, #2e1065 0%, #0f172a 60%); }
html.standalone main { height: auto; min-height: 100vh; }
html.padded main { padding: 20px; }
main { display: flex; flex-direction: column; gap: 10px; height: 100vh; padding: 6px; overflow: auto;
  scrollbar-width: thin; scrollbar-color: rgba(148, 163, 184, 0.35) transparent; }
.kpis { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 10px; }
.kpi, figure { background: linear-gradient(160deg, rgba(255, 255, 255, 0.07), rgba(255, 255, 255, 0.025));
  border: 1px solid rgba(255, 255, 255, 0.07); border-radius: 10px; padding: 9px 12px;
  box-shadow: 0 1px 0 rgba(255, 255, 255, 0.05) inset, 0 6px 18px rgba(0, 0, 0, 0.12); }
.kpi { position: relative; display: flex; flex-direction: column; min-width: 0; overflow: hidden; }
.kpi::before { content: ""; position: absolute; left: 0; top: 10px; bottom: 10px; width: 3px; border-radius: 0 3px 3px 0; background: var(--accent, var(--muted)); }
.ratio { --accent: var(--violet); } .up { --accent: var(--blue); } .down { --accent: var(--rose); }
.zen { --accent: var(--amber); } .seeds { --accent: var(--green); } .grade { --accent: var(--orange); }
.label, figcaption, h3 { color: var(--muted); font-size: 11px; font-weight: 500; text-transform: uppercase; letter-spacing: 0.04em; margin: 0; }
.value { font-size: 26px; font-weight: 700; letter-spacing: -0.01em; font-variant-numeric: tabular-nums; line-height: 1.15; white-space: nowrap; }
.sub { color: var(--muted); font-size: 12px; min-height: 16px; }
.ratio .value { color: var(--violet); } .zen .value { color: var(--amber); } .seeds .value { color: var(--green); } .grade .value { color: var(--orange); }
.charts, .bottom, .quests { display: grid; grid-template-columns: repeat(auto-fit, minmax(260px, 1fr)); gap: 10px; }
.charts { flex: 1; min-height: 140px; }
figure { margin: 0; display: flex; flex-direction: column; gap: 6px; min-width: 0; min-height: 0; }
figcaption { display: flex; gap: 10px; flex-wrap: wrap; align-items: baseline; }
.key { text-transform: none; letter-spacing: 0; font-size: 11px; }
.key::before { content: ""; display: inline-block; width: 8px; height: 8px; border-radius: 2px; margin-right: 4px; background: currentColor; }
.chart { position: relative; flex: 1; min-height: 70px; border-bottom: 1px solid var(--line);
  background: repeating-linear-gradient(to bottom, transparent 0, transparent calc(25% - 1px), rgba(148, 163, 184, 0.07) calc(25% - 1px), rgba(148, 163, 184, 0.07) 25%); }
.chart svg { position: absolute; inset: 0; width: 100%; height: 100%; overflow: visible; }
.mark { position: absolute; font-size: 10px; color: var(--muted); transform: translateY(-100%); white-space: nowrap; }
.mark.right { right: 0; } .mark.left { left: 0; }
.axis { display: flex; justify-content: space-between; color: var(--muted); font-size: 10px; }
.line { fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; vector-effect: non-scaling-stroke; }
polygon.area { stroke: none; fill-opacity: 0.14; }
.area.c-ratio { fill: var(--violet); } .area.c-balance { fill: var(--green); } .area.c-up { fill: var(--blue); } .area.c-down { fill: var(--rose); }
.dot { stroke-width: 6; stroke-linecap: round; vector-effect: non-scaling-stroke; }
.guide { stroke: var(--muted); stroke-width: 1; stroke-dasharray: 4 4; vector-effect: non-scaling-stroke; }
.c-ratio { stroke: var(--violet); color: var(--violet); } .c-balance { stroke: var(--green); color: var(--green); }
.c-paid { fill: var(--amber); fill-opacity: 0.85; color: var(--amber); } .c-up { stroke: var(--blue); color: var(--blue); }
.c-down { stroke: var(--rose); color: var(--rose); }
.gauge { position: relative; height: 14px; border-radius: 7px; overflow: hidden; display: flex; margin: 18px 0 20px; }
.gauge span { display: block; height: 100%; opacity: 0.75; }
.ticks { position: relative; height: 0; }
.tick { position: absolute; top: -40px; transform: translateX(-50%); font-size: 10px; color: var(--muted); white-space: nowrap; text-align: center; }
.tick::after { content: ""; display: block; width: 1px; height: 22px; margin: 2px auto 0; background: rgba(226, 232, 240, 0.5); }
.cursor { position: absolute; top: -26px; transform: translateX(-50%); font-weight: 700; color: var(--text); font-size: 12px; }
.cursor::after { content: ""; display: block; width: 0; height: 0; margin: 1px auto 0; border: 6px solid transparent; border-top-color: var(--text); }
.gaps { display: flex; gap: 18px; flex-wrap: wrap; font-size: 12px; color: var(--muted); }
.gaps b { color: var(--text); font-weight: 600; }
.rows { display: flex; flex-direction: column; gap: 7px; }
.row { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 2px 10px; align-items: center; }
.row .what { min-width: 0; }
.row .title { font-weight: 600; }
.row .hint { color: var(--muted); font-size: 11px; }
.bar { grid-column: 1 / -1; height: 6px; border-radius: 3px; background: var(--line); overflow: hidden; }
.bar span { display: block; height: 100%; background: var(--green); }
.bar.done span { background: var(--violet); }
.count { font-variant-numeric: tabular-nums; font-weight: 600; white-space: nowrap; }
.side { display: flex; gap: 8px; align-items: center; }
form { margin: 0; display: inline; }
.switch { font: inherit; font-size: 11px; font-weight: 600; border-radius: 999px; padding: 3px 10px 3px 8px; cursor: pointer;
  border: 1px solid rgba(255, 255, 255, 0.12); background: rgba(148, 163, 184, 0.12); color: var(--muted); white-space: nowrap; }
.switch::before { content: ""; display: inline-block; width: 7px; height: 7px; border-radius: 50%; margin-right: 6px; background: currentColor; }
.switch.on { background: rgba(52, 211, 153, 0.16); color: var(--green); border-color: rgba(52, 211, 153, 0.4); }
.tag { font-size: 11px; color: var(--muted); white-space: nowrap; }
.voies { display: flex; gap: 14px; flex-wrap: wrap; font-size: 12px; color: var(--muted); }
.cats { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 7px 16px; }
.settings { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 6px 16px; }
.settings form { display: flex; gap: 6px; align-items: center; justify-content: space-between; }
.settings input { width: 76px; font: inherit; color: var(--text); background: rgba(15, 23, 42, 0.6); border: 1px solid var(--line); border-radius: 6px; padding: 2px 6px; }
.settings button.save { font: inherit; font-size: 11px; color: var(--text); background: rgba(167, 139, 250, 0.2); border: 1px solid rgba(167, 139, 250, 0.4); border-radius: 6px; padding: 2px 8px; cursor: pointer; }
.toggles { display: flex; flex-wrap: wrap; gap: 6px; }
.disk { display: flex; gap: 14px; align-items: center; min-height: 0; }
.donut { position: relative; width: 118px; height: 118px; flex: none; }
.donut svg { width: 100%; height: 100%; transform: rotate(-90deg); }
.donut circle { fill: none; stroke-width: 0.3; }
.donut .center { position: absolute; inset: 0; display: flex; flex-direction: column; align-items: center; justify-content: center; text-align: center; font-size: 11px; color: var(--muted); }
.donut .center b { color: var(--green); font-size: 20px; font-weight: 600; line-height: 1.1; }
.legend { list-style: none; margin: 0; padding: 0; font-size: 12px; display: flex; flex-direction: column; gap: 2px; min-width: 0; flex: 1; }
.legend li { display: grid; grid-template-columns: auto 1fr auto; align-items: baseline; gap: 0 6px; min-width: 0; }
.swatch { width: 8px; height: 8px; border-radius: 2px; flex: none; }
.earned table { width: 100%; margin-top: 6px; border-collapse: collapse; font-size: 12px; }
.earned th { color: var(--muted); font-size: 11px; font-weight: 500; text-align: right; padding: 0 0 3px 6px; }
.earned td { text-align: right; padding: 3px 0 0 6px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.earned th:first-child, .earned td:first-child { text-align: left; padding-left: 0; }
.earned td.week { text-align: left; white-space: normal; color: var(--muted); font-size: 11px; padding: 0 0 3px 14px; }
.earned tfoot td { font-weight: 600; }
.earned .swatch { display: inline-block; margin-right: 6px; }
.name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.figure { font-variant-numeric: tabular-nums; font-weight: 600; white-space: nowrap; }
.meta { grid-column: 2 / -1; color: var(--muted); font-size: 11px; }
.empty { color: var(--muted); }
@media (max-width: 640px) {
  main { height: auto; overflow: visible; }
  .kpis { grid-template-columns: repeat(2, minmax(0, 1fr)); }
  .charts { flex: none; }
  .chart { flex: none; height: 140px; }
}
"""
SCRIPT = (
    'if (parent !== window) { const main = document.querySelector("main"); '
    'new ResizeObserver(() => parent.postMessage({ratioBoardHeight: main.scrollHeight}, "*")).observe(main); }'
)
SCRIPT_HASH = base64.b64encode(hashlib.sha256(SCRIPT.encode()).digest()).decode()


def escape(text):
    return html.escape(str(text))


def figure_text(value, digits, words):
    return "-" if value is None else f"{value:,.{digits}f}".replace(",", " ").replace(".", words["decimal"])


def size_text(value, words, digits=1):
    return f"{figure_text(value, digits, words)} {words['gb']}"


def window_of(history):
    end = datetime.date.fromisoformat(history[-1]["day"])
    start = end - datetime.timedelta(days=HISTORY_DAYS - 1)
    entries = [e for e in history if e["day"] >= start.isoformat()]
    offsets = [(datetime.date.fromisoformat(e["day"]) - start).days for e in entries]
    return start, end, entries, offsets


def x_of(offset):
    return (offset + 0.5) * 100 / HISTORY_DAYS


def y_of(value, top):
    return 100 - 90 * value / top


def top_of(values):
    found = max([v for v in values if v is not None] or [0])
    return found if found > 0 else 1


def lines(css, offsets, values, top):
    parts, run = [], []
    for at, value in [*zip(offsets, values), (None, None)]:
        if run and (value is None or at > run[-1][0] + 1):
            if len(run) == 1:
                x, y = x_of(run[0][0]), y_of(run[0][1], top)
                parts.append(f'<line class="dot {css}" x1="{x:.2f}" y1="{y:.2f}" x2="{x:.2f}" y2="{y:.2f}"/>')
            else:
                points = " ".join(f"{x_of(a):.2f},{y_of(v, top):.2f}" for a, v in run)
                base = f"{x_of(run[-1][0]):.2f},100 {x_of(run[0][0]):.2f},100"
                parts.append(f'<polygon class="area {css}" points="{points} {base}"/>')
                parts.append(f'<polyline class="line {css}" points="{points}"/>')
            run = []
        if value is not None:
            run.append((at, value))
    return parts


def bars(css, offsets, values, top):
    width = 100 / HISTORY_DAYS * 0.6
    return [
        f'<rect class="{css}" x="{x_of(at) - width / 2:.2f}" y="{y_of(v, top):.2f}" '
        f'width="{width:.2f}" height="{100 - y_of(v, top):.2f}"/>'
        for at, v in zip(offsets, values)
        if v and v > 0
    ]


def chart(caption, keys, shapes, marks, start, end):
    svg = '<svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">' + "".join(shapes) + "</svg>"
    legend = "".join(f'<span class="key {css}">{escape(text)}</span>' for css, text in keys)
    labels = "".join(f'<span class="mark {side}" style="top:{top:.1f}%">{escape(text)}</span>' for side, top, text in marks)
    return (
        f"<figure><figcaption>{escape(caption)}{legend}</figcaption>"
        f'<div class="chart">{svg}{labels}</div>'
        f'<div class="axis"><span>{start.strftime("%d/%m")}</span><span>{end.strftime("%d/%m")}</span></div></figure>'
    )


def kpi(css, label, value, sub=""):
    return (
        f'<div class="kpi {css}"><span class="label">{escape(label)}</span>'
        f'<span class="value">{escape(value)}</span><span class="sub">{escape(sub)}</span></div>'
    )


def grade_text(grade, words):
    if not grade or not grade[0]:
        return "-"
    roman = {1: "I", 2: "II", 3: "III", 4: "IV"}.get(grade[1], str(grade[1]))
    return f"{words['grades'].get(grade[0], grade[0].title())} {roman}"


def strip(board, words):
    account = board.get("account") or {}
    today = board["history"][-1] if board.get("history") else {}
    actual, credit = account.get("actual_gb"), account.get("credit_gb")
    paid, day = today.get("zen_paid"), today.get("downloaded_day_gb")
    seeds = board.get("seeds")
    return "".join([
        kpi("ratio", words["ratio"], figure_text(account.get("ratio"), 2, words),
            words["floor"].format(v=figure_text(account.get("floor"), 2, words))),
        kpi("up", words["uploaded"], size_text(account.get("uploaded_gb"), words),
            "" if actual is None else words["credit"].format(v=size_text(actual, words), c=size_text(credit, words))),
        kpi("down", words["downloaded"], size_text(account.get("downloaded_gb"), words),
            "" if day is None else words["day"].format(v=size_text(day, words))),
        kpi("zen", words["zen"], figure_text(account.get("zen"), 0, words),
            words["paid_last"].format(v=figure_text(paid, 0, words)) if paid else ""),
        kpi("seeds", words["seeds"], figure_text(seeds, 0, words),
            "" if seeds is None else words["known"].format(v=figure_text(board.get("known"), 0, words))),
        kpi("grade", words["grade"], grade_text(account.get("grade"), words), account.get("class") or ""),
    ])


def gauge(board, words):
    tiers = board.get("tiers") or []
    ratio = (board.get("account") or {}).get("ratio")
    if not tiers:
        return ""
    top = max(2.4, tiers[-1]["ratio"] * 1.2, (ratio or 0) * 1.1)
    edges = [0] + [t["ratio"] for t in tiers] + [top]
    bands = "".join(
        f'<span style="width:{100 * (high - low) / top:.2f}%;background:{TIER_COLORS[i % len(TIER_COLORS)]}"></span>'
        for i, (low, high) in enumerate(itertools.pairwise(edges))
    )
    ticks = "".join(
        f'<span class="tick" style="left:{100 * t["ratio"] / top:.2f}%">{escape(figure_text(t["ratio"], 1, words))}<br>'
        f'{escape(words["tier_names"].get(t["name"], t["name"]))}</span>'
        for t in tiers
    )
    cursor = "" if ratio is None else f'<span class="cursor" style="left:{min(100 * ratio / top, 100):.2f}%">{escape(figure_text(ratio, 2, words))}</span>'
    above = next((t for t in tiers if t["gap_gb"] is not None and t["gap_gb"] > 0), None)
    below = next((t for t in reversed(tiers) if t["gap_gb"] is not None and t["gap_gb"] <= 0), None)
    gaps = []
    if ratio is not None:
        if above:
            gaps.append(words["above"].format(
                r=figure_text(above["ratio"], 1, words), n=words["tier_names"].get(above["name"], above["name"]),
                v=size_text(above["gap_gb"], words)))
        else:
            gaps.append(words["top"])
        if below:
            gaps.append(words["below"].format(
                r=figure_text(below["ratio"], 1, words), n=words["tier_names"].get(below["name"], below["name"]),
                v=size_text(-below["gap_gb"], words)))
    return (
        f'<figure class="tiers"><figcaption>{escape(words["tiers"])}</figcaption>'
        f'<div class="gauge">{bands}</div><div class="ticks">{ticks}{cursor}</div>'
        f'<div class="gaps">{"".join(f"<span>{escape(g)}</span>" for g in gaps)}</div></figure>'
    )


def trends(board, words):
    history = board["history"]
    start, end, entries, offsets = window_of(history)
    tiers = board.get("tiers") or []
    ratios = [e.get("ratio") for e in entries]
    top = top_of(ratios + [t["ratio"] for t in tiers]) * 1.05
    guides = [f'<line class="guide" x1="0" y1="{y_of(t["ratio"], top):.2f}" x2="100" y2="{y_of(t["ratio"], top):.2f}"/>' for t in tiers]
    ratio_chart = chart(
        words["ratio"], [], [*guides, *lines("c-ratio", offsets, ratios, top)],
        [("right", y_of(t["ratio"], top), figure_text(t["ratio"], 1, words)) for t in tiers],
        start, end,
    )
    paid_values = [e.get("zen_paid") for e in entries]
    balance = [e.get("zen") for e in entries]
    zen_chart = chart(
        words["zen_chart"], [("c-paid", words["paid"]), ("c-balance", words["balance"])],
        bars("c-paid", offsets, paid_values, top_of(paid_values)) + lines("c-balance", offsets, balance, top_of(balance)),
        [("left", 10, figure_text(top_of(paid_values), 0, words)), ("right", 10, figure_text(top_of(balance), 0, words))],
        start, end,
    )
    up = [e.get("uploaded_gb") for e in entries]
    down = [e.get("downloaded_gb") for e in entries]
    both = top_of(up + down)
    transfer_chart = chart(
        words["transfer"], [("c-up", words["uploaded"]), ("c-down", words["downloaded"])],
        lines("c-up", offsets, up, both) + lines("c-down", offsets, down, both),
        [("left", 10, size_text(both, words, 0))],
        start, end,
    )
    return f'<section class="charts">{ratio_chart}{zen_chart}{transfer_chart}</section>'


def switch(name, on, label, words, part, language):
    state = "on" if on else "off"
    return (
        f'<form method="post" action="/controls"><input type="hidden" name="name" value="{escape(name)}">'
        f'<input type="hidden" name="value" value="{"off" if on else "on"}">'
        f'<input type="hidden" name="part" value="{part}"><input type="hidden" name="lang" value="{language}">'
        f'<button class="switch {state}" type="submit" aria-pressed="{str(on).lower()}">{escape(label or words[state])}</button></form>'
    )


def progress_bar(current, target, done=False):
    share = 100 if not target else min(100 * (current or 0) / target, 100)
    return f'<div class="bar{" done" if done else ""}"><span style="width:{share:.1f}%"></span></div>'


def quests(board, words, language):
    switches = (board.get("controls") or {}).get("switches", {})
    account = board.get("account") or {}
    voies = "".join(
        f'<span><b>{escape(words["voie"].get(voie, voie))}</b> {escape(words["voie_done"].format(d=v["done"], t=v["total"], c=figure_text((account.get("voies") or {}).get(voie), 0, words)))}</span>'
        for voie, v in (board.get("voies") or {}).items()
    )
    rows = []
    for c in board.get("challenges", []):
        level = words["finished"] if c["done"] else words["level"].format(l=c["level"], n=c["levels"])
        side = (
            switch(c["switch"], switches.get(c["switch"], False), "", words, "2", language)
            if c["switch"] else f'<span class="tag">{escape(words["indicator"])}</span>'
        )
        rows.append(
            f'<div class="row"><div class="what"><div class="title">{escape(words["names"].get(c["key"], c["key"]))}</div>'
            f'<div class="hint">{escape(level)}, {escape(c["description"])}</div></div>'
            f'<div class="side"><span class="count">{c["current"]} / {c["target"]}</span>{side}</div>'
            f'{progress_bar(c["current"], c["target"], c["done"])}</div>'
        )
    cats = []
    for c in board.get("categories", []):
        cats.append(
            f'<div class="row"><div class="what title">{escape(c["name"])}</div>'
            f'<div class="side"><span class="count">{c["count"]} / {c["target"]}</span>'
            f'{switch(c["switch"], switches.get(c["switch"], False), "", words, "2", language)}</div>'
            f'{progress_bar(c["count"], c["target"], c["count"] >= c["target"])}</div>'
        )
    return (
        f'<section class="quests"><figure><figcaption>{escape(words["challenges"])}</figcaption>'
        f'<div class="voies">{voies}</div><div class="rows">{"".join(rows)}</div></figure>'
        f'<figure><figcaption>{escape(words["categories"])}</figcaption><div class="cats">{"".join(cats)}</div></figure></section>'
    )


def settings(board, words, language):
    controls = board.get("controls") or {}
    switches = controls.get("switches", {})
    toggles = "".join(
        switch(name, switches.get(name, False), words["switches"][name], words, "3", language) for name in BEHAVIOURS if name in switches
    )
    numbers = "".join(
        f'<form method="post" action="/controls"><label for="n-{name}">{escape(words["numbers"][name])}</label><span class="side">'
        f'<input id="n-{name}" type="number" name="value" step="{NUMBER_STEPS[name]}" min="{NUMBER_LIMITS[name][0]}" '
        f'max="{NUMBER_LIMITS[name][1]}" value="{escape(value)}">'
        f'<input type="hidden" name="name" value="{name}"><input type="hidden" name="part" value="3">'
        f'<input type="hidden" name="lang" value="{language}"><button class="save" type="submit">{escape(words["save"])}</button></span></form>'
        for name, value in controls.get("numbers", {}).items()
        if name in NUMBER_STEPS
    )
    return (
        f'<figure><figcaption>{escape(words["behaviours"])}</figcaption><div class="toggles">{toggles}</div>'
        f'<div class="settings">{numbers}</div></figure>'
    )


def donut(farm, words):
    budget = farm["budget_gb"] or 1
    used = sum(s["gb"] for s in farm["slices"])
    start, rings, legend = 0, [], []
    for s in farm["slices"]:
        part = 100 * s["gb"] / max(budget, used)
        color = COLORS.get(s["source"], COLORS["unknown"])
        rings.append(
            f'<circle r="0.84" pathLength="100" stroke="{color}" '
            f'stroke-dasharray="{part:.2f} {100 - part:.2f}" stroke-dashoffset="{-start:.2f}"/>'
        )
        legend.append(
            f'<li><span class="swatch" style="background:{color}"></span>'
            f'<span class="name">{escape(s["source"])}</span>'
            f'<span class="figure">{escape(size_text(s["gb"], words))}</span>'
            f'<span class="meta">{escape(words["explain"].get(s["source"], ""))}</span></li>'
        )
        start += part
    legend.append(
        f'<li><span class="swatch" style="background:var(--line)"></span><span class="name">{escape(words["free"])}</span>'
        f'<span class="figure">{escape(size_text(max(budget - used, 0), words))}</span></li>'
    )
    center = words["of"].format(budget=figure_text(budget, 0, words), gb=words["gb"])
    svg = '<svg viewBox="-1 -1 2 2" aria-hidden="true"><circle r="0.84" stroke="rgba(148,163,184,0.22)"/>' + "".join(rings) + "</svg>"
    return (
        f'<figure><figcaption>{escape(words["disk"])}</figcaption><div class="disk">'
        f'<div class="donut">{svg}<div class="center"><b>{figure_text(100 * used / budget, 0, words)} %</b>'
        f'{escape(center)}</div></div><ul class="legend">{"".join(legend)}</ul></div></figure>'
    )


def earned_rows(name, swatch, f, words):
    cells = [str(f["torrents"]), size_text(f["downloaded_gb"], words), size_text(f["uploaded_gb"], words), figure_text(f["ratio"], 2, words)]
    week = words["week"].format(
        down=size_text(f["week_downloaded_gb"], words), up=size_text(f["week_uploaded_gb"], words),
        ratio=figure_text(f["week_ratio"], 2, words), days=figure_text(f["farm_days"], 1, words),
    )
    return (
        f'<tr><td>{swatch}{escape(name)}</td>{"".join(f"<td>{escape(c)}</td>" for c in cells)}</tr>'
        f'<tr><td class="week" colspan="5">{escape(week)}</td></tr>'
    )


def share(uploaded, downloaded):
    return round(uploaded / downloaded, 2) if downloaded else None


def total_of(sources):
    keys = ("torrents", "downloaded_gb", "uploaded_gb", "week_downloaded_gb", "week_uploaded_gb", "farm_days")
    sums = {k: sum(f[k] for f in sources) for k in keys}
    return {**sums, "ratio": share(sums["uploaded_gb"], sums["downloaded_gb"]), "week_ratio": share(sums["week_uploaded_gb"], sums["week_downloaded_gb"])}


def earned(sources, words):
    head = "".join(f"<th>{escape(c)}</th>" for c in words["columns"])
    body = "".join(
        earned_rows(f["source"], f'<span class="swatch" style="background:{COLORS.get(f["source"], COLORS["unknown"])}"></span>', f, words)
        for f in sources
    )
    foot = earned_rows(words["total"], "", total_of(sources), words) if sources else ""
    return (
        f'<figure class="earned"><figcaption>{escape(words["earned"])}</figcaption>'
        f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody><tfoot>{foot}</tfoot></table></figure>"
    )


def pixels(text):
    return int(text) if text.isdigit() and 0 < int(text) <= 4000 else None


def body_of(board, part, words, language):
    if not board or not board.get("history"):
        return f'<p class="empty">{escape(words["empty"])}</p>'
    first = f'<section class="kpis">{strip(board, words)}</section>{gauge(board, words)}{trends(board, words)}'
    second = quests(board, words, language)
    third = f'{settings(board, words, language)}<section class="bottom">{donut(board["farm"], words)}{earned(board.get("sources", []), words)}</section>'
    return {"1": first, "2": second, "3": third}.get(part, first + second + third)


def board_page(board, part=None, language="en", height=None):
    language = language if language in TEXT else "en"
    words = TEXT[language]
    part = part if part in PARTS else None
    classes = " ".join(c for c, on in (("standalone", not part), ("padded", height)) if on)
    fixed = f"html.padded main {{ height: {height}px; }}" if height else ""
    return (
        f'<!doctype html><html lang="{language}"{f' class="{classes}"' if classes else ""}><head><meta charset="utf-8">'
        '<meta name="viewport" content="width=device-width, initial-scale=1">'
        f"<title>Farm</title><style>{STYLE}{fixed}</style></head><body><main>{body_of(board, part, words, language)}</main>"
        f"<script>{SCRIPT}</script></body></html>"
    )
