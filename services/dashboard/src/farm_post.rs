use jiff::ZonedDateTime;
use jiff::civil::Date;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

const API: &str = "https://discord.com/api/v10/webhooks";
const USER_AGENT: &str = "DiscordBot (https://github.com/Amaroke/Jellynas, 1.0)";
const VIOLET: u32 = 0xA78BFA;
const ROSE: u32 = 0xFB7185;
const HISTORY_DAYS: i64 = 30;
const WIDTH: f64 = 900.0;
const PAD: f64 = 24.0;
const GAP: f64 = 10.0;
const SCALE: f32 = 2.0;
const TEXT: &str = "#e2e8f0";
const MUTED: &str = "#94a3b8";
const LINE: &str = "rgba(148,163,184,0.22)";
const TIER_COLORS: [&str; 5] = ["#fb7185", "#fbbf24", "#facc15", "#34d399", "#a78bfa"];
const REGULAR: &[u8] = include_bytes!("fonts/plex-sans-regular.ttf");
const SEMIBOLD: &[u8] = include_bytes!("fonts/plex-sans-semibold.ttf");
const IMAGE: &str = "farm.png";

pub type Send<'a> = dyn Fn(&str, &str, Vec<u8>, &str) -> Result<(u16, Value), String> + 'a;

pub struct Poster {
    url: String,
    state: PathBuf,
    last: Mutex<Option<(String, String)>>,
}

impl Poster {
    pub fn new(id: &str, token: &str, state: PathBuf) -> Option<Poster> {
        let (id, token) = (id.trim(), token.trim());
        if id.is_empty() || token.is_empty() {
            return None;
        }
        Some(Poster {
            url: format!("{API}/{id}/{token}"),
            last: Mutex::new(Self::saved(&state)),
            state,
        })
    }

    fn saved(state: &std::path::Path) -> Option<(String, String)> {
        let saved: Value = serde_json::from_slice(&fs::read(state).ok()?).ok()?;
        Some((
            saved["day"].as_str()?.to_string(),
            saved["message_id"].as_str()?.to_string(),
        ))
    }

    pub fn post(&self, board: &Value, now: &ZonedDateTime) -> Result<String, String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build()
            .into();
        let send = |method: &str, path: &str, body: Vec<u8>, kind: &str| {
            let url = format!("{}{path}", self.url);
            let request = ureq::http::Request::builder()
                .method(method)
                .uri(&url)
                .header("Content-Type", kind)
                .header("User-Agent", USER_AGENT)
                .body(body)
                .map_err(|error| error.to_string())?;
            let mut response = agent.run(request).map_err(|error| error.to_string())?;
            let status = response.status().as_u16();
            let text = response
                .body_mut()
                .read_to_string()
                .map_err(|error| error.to_string())?;
            Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
        };
        self.publish(board, now, &send)
    }

    pub fn publish(
        &self,
        board: &Value,
        now: &ZonedDateTime,
        send: &Send,
    ) -> Result<String, String> {
        let image = png_of(&svg_of(board))?;
        let (body, kind) = multipart(&payload(board, now), &image);
        let day = now.date().to_string();
        let mut last = self
            .last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((saved_day, id)) = last.as_ref()
            && *saved_day == day
        {
            let (status, answer) = send("PATCH", &format!("/messages/{id}"), body.clone(), &kind)?;
            match status {
                200..=299 => return Ok(format!("Farm board edited, message {id} of {day}")),
                404 => {}
                _ => return Err(format!("Discord answered {status}: {answer}")),
            }
        }
        let (status, answer) = send("POST", "?wait=true", body, &kind)?;
        let id = match (status, answer["id"].as_str()) {
            (200..=299, Some(id)) => id.to_string(),
            _ => return Err(format!("Discord answered {status}: {answer}")),
        };
        *last = Some((day.clone(), id.clone()));
        let state = json!({"day": day, "message_id": id});
        let kept = match fs::write(&self.state, state.to_string()) {
            Ok(()) => String::new(),
            Err(error) => format!(", but cannot save {}: {error}", self.state.display()),
        };
        Ok(format!("Farm board posted, message {id} of {day}{kept}"))
    }
}

pub fn until_next_hour(now: &ZonedDateTime) -> std::time::Duration {
    let into = u64::from(now.minute().unsigned_abs()) * 60 + u64::from(now.second().unsigned_abs());
    std::time::Duration::from_secs(3600 - into.min(3599) + 30)
}

pub fn ready(board: &Value) -> bool {
    !board["updated_at"].is_null()
        && board["history"]
            .as_array()
            .is_some_and(|days| !days.is_empty())
}

pub fn tier_of(board: &Value) -> Option<(usize, f64)> {
    let ratio = board["account"]["ratio"].as_f64()?;
    let reached = board["tiers"]
        .as_array()?
        .iter()
        .filter(|tier| tier["ratio"].as_f64().is_some_and(|tier| ratio >= tier))
        .count();
    Some((
        reached,
        board["account"]["floor"].as_f64().unwrap_or_default(),
    ))
}

pub fn crossed(before: Option<(usize, f64)>, after: Option<(usize, f64)>) -> bool {
    matches!((before, after), (Some((a, floor)), Some((b, now))) if floor == now && a != b)
}

fn payload(board: &Value, now: &ZonedDateTime) -> Value {
    let paused = board["account"]["paused"].as_bool().unwrap_or(false);
    json!({
        "embeds": [{
            "color": if paused { ROSE } else { VIOLET },
            "image": {"url": format!("attachment://{IMAGE}")},
            "timestamp": now.timestamp().strftime("%Y-%m-%dT%H:%M:%S+00:00").to_string(),
        }],
        "attachments": [{"id": 0, "filename": IMAGE}],
    })
}

pub fn multipart(payload: &Value, image: &[u8]) -> (Vec<u8>, String) {
    let boundary = format!("{:032x}", boundary_of(image));
    let mut body = Vec::new();
    let _ = write!(
        Bytes(&mut body),
        "--{boundary}\r\nContent-Disposition: form-data; name=\"payload_json\"\r\nContent-Type: application/json\r\n\r\n{payload}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"files[0]\"; filename=\"{IMAGE}\"\r\nContent-Type: image/png\r\n\r\n"
    );
    body.extend_from_slice(image);
    let _ = write!(Bytes(&mut body), "\r\n--{boundary}--\r\n");
    (body, format!("multipart/form-data; boundary={boundary}"))
}

struct Bytes<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Bytes<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0.extend_from_slice(text.as_bytes());
        Ok(())
    }
}

fn boundary_of(seed: &[u8]) -> u128 {
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    seed.iter()
        .fold(time ^ 0x9e37_79b9_7f4a_7c15, |hash, byte| {
            (hash ^ u128::from(*byte)).wrapping_mul(0x0100_0000_0100_0000_0000_0000_013b)
        })
}

pub fn png_of(svg: &str) -> Result<Vec<u8>, String> {
    let mut options = resvg::usvg::Options::default();
    let fonts = options.fontdb_mut();
    fonts.load_font_data(REGULAR.to_vec());
    fonts.load_font_data(SEMIBOLD.to_vec());
    fonts.set_sans_serif_family("IBM Plex Sans");
    let tree = resvg::usvg::Tree::from_str(svg, &options).map_err(|error| error.to_string())?;
    let size = tree.size();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(
        (size.width() * SCALE).ceil() as u32,
        (size.height() * SCALE).ceil() as u32,
    )
    .ok_or("the Farm board has no size")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(SCALE, SCALE),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().map_err(|error| error.to_string())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn figure(value: Option<f64>, digits: usize) -> String {
    let Some(value) = value else {
        return "-".into();
    };
    let text = format!("{:.*}", digits, value.abs());
    let (whole, part) = text.split_once('.').unwrap_or((&text, ""));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(' ');
        }
        grouped.push(digit);
    }
    let sign = if value < 0.0 && text.chars().any(|c| c.is_ascii_digit() && c != '0') {
        "-"
    } else {
        ""
    };
    if part.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped},{part}")
    }
}

fn size(value: Option<f64>, digits: usize) -> String {
    format!("{} Go", figure(value, digits))
}

struct Svg(String);

impl Svg {
    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        x: f64,
        y: f64,
        size: f64,
        weight: u32,
        color: &str,
        anchor: &str,
        text: &str,
    ) {
        let _ = write!(
            self.0,
            r#"<text x="{x:.1}" y="{y:.1}" font-size="{size}" font-weight="{weight}" fill="{color}" text-anchor="{anchor}">{}</text>"#,
            escape(text)
        );
    }

    fn label(&mut self, x: f64, y: f64, text: &str) {
        self.text(x, y, 11.0, 500, MUTED, "start", &text.to_uppercase());
    }

    fn card(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let _ = write!(
            self.0,
            r#"<rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="10" fill="rgba(255,255,255,0.05)" stroke="rgba(255,255,255,0.07)"/>"#
        );
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, rx: f64, fill: &str) {
        let _ = write!(
            self.0,
            r#"<rect x="{x:.2}" y="{y:.2}" width="{w:.2}" height="{h:.2}" rx="{rx}" fill="{fill}"/>"#
        );
    }
}

fn kpis(svg: &mut Svg, board: &Value, top: f64) -> f64 {
    let account = &board["account"];
    let today = board["history"]
        .as_array()
        .and_then(|days| days.last())
        .cloned()
        .unwrap_or(Value::Null);
    let day = |key: &str| {
        today[key]
            .as_f64()
            .map(|value| format!("{} en 24 h", size(Some(value), 1)))
            .unwrap_or_default()
    };
    let cards = [
        (
            "#a78bfa",
            "Ratio du compte",
            figure(account["ratio"].as_f64(), 2),
            "#a78bfa",
            String::new(),
        ),
        (
            "#60a5fa",
            "Envoyé",
            size(account["uploaded_gb"].as_f64(), 1),
            TEXT,
            day("uploaded_day_gb"),
        ),
        (
            "#fb7185",
            "Reçu",
            size(account["downloaded_gb"].as_f64(), 1),
            TEXT,
            day("downloaded_day_gb"),
        ),
        (
            "#fbbf24",
            "Zen",
            figure(account["zen"].as_f64(), 0),
            "#fbbf24",
            String::new(),
        ),
        (
            "#34d399",
            "Seeds sur C411",
            figure(board["seeds"].as_f64(), 0),
            "#34d399",
            String::new(),
        ),
        (
            "#fb923c",
            "Go effectifs",
            size(account["available_gb"].as_f64(), 0),
            "#fb923c",
            String::new(),
        ),
    ];
    let width = (WIDTH - 2.0 * PAD - 2.0 * GAP) / 3.0;
    let height = 80.0;
    for (index, (accent, label, value, color, sub)) in cards.iter().enumerate() {
        let x = PAD + (index % 3) as f64 * (width + GAP);
        let y = top + (index / 3) as f64 * (height + GAP);
        svg.card(x, y, width, height);
        svg.rect(x, y + 10.0, 3.0, height - 20.0, 1.5, accent);
        svg.label(x + 14.0, y + 22.0, label);
        svg.text(x + 14.0, y + 52.0, 26.0, 700, color, "start", value);
        svg.text(x + 14.0, y + 70.0, 12.0, 400, MUTED, "start", sub);
    }
    top + 2.0 * height + GAP
}

fn tier_name(name: &str) -> &str {
    match name {
        "blocked" => "bloqué",
        "even" => "équilibre",
        "floor" => "plancher",
        "goal" => "objectif",
        other => other,
    }
}

fn gauge(svg: &mut Svg, board: &Value, top: f64) -> f64 {
    let tiers = board["tiers"].as_array().cloned().unwrap_or_default();
    if tiers.is_empty() {
        return top - GAP;
    }
    let ratio = board["account"]["ratio"].as_f64();
    let last = tiers
        .last()
        .and_then(|tier| tier["ratio"].as_f64())
        .unwrap_or(2.0);
    let low = 0.6_f64.min(ratio.unwrap_or(1.0) * 0.9);
    let high = 2.2_f64.max(last * 1.1).max(ratio.unwrap_or(0.0) * 1.05);
    let height = 112.0 + 18.0 * tiers.len() as f64;
    let width = WIDTH - 2.0 * PAD;
    svg.card(PAD, top, width, height);
    svg.label(PAD + 12.0, top + 22.0, "Paliers de ratio");
    let (left, span, bar) = (PAD + 20.0, width - 40.0, top + 58.0);
    let at = |value: f64| left + span * ((value - low) / (high - low)).clamp(0.0, 1.0);
    let _ = write!(
        svg.0,
        r#"<clipPath id="gauge"><rect x="{left:.2}" y="{bar:.2}" width="{span:.2}" height="12" rx="6"/></clipPath><g clip-path="url(#gauge)" opacity="0.8">"#
    );
    let mut edges = vec![low];
    edges.extend(tiers.iter().filter_map(|tier| tier["ratio"].as_f64()));
    edges.push(high);
    for (index, pair) in edges.windows(2).enumerate() {
        let (start, end) = (at(pair[0]), at(pair[1]));
        svg.rect(
            start,
            bar,
            (end - start).max(0.0),
            12.0,
            0.0,
            TIER_COLORS[index % 5],
        );
    }
    svg.0.push_str("</g>");
    for tier in &tiers {
        let Some(value) = tier["ratio"].as_f64() else {
            continue;
        };
        let x = at(value);
        let _ = write!(
            svg.0,
            r#"<line x1="{x:.2}" y1="{:.2}" x2="{x:.2}" y2="{:.2}" stroke="rgba(226,232,240,0.6)"/>"#,
            bar - 2.0,
            bar + 16.0
        );
        svg.text(
            x,
            bar + 30.0,
            10.0,
            400,
            MUTED,
            "middle",
            &figure(Some(value), 1),
        );
    }
    if let Some(ratio) = ratio {
        let x = at(ratio);
        svg.text(
            x,
            bar - 14.0,
            12.0,
            700,
            TEXT,
            "middle",
            &figure(Some(ratio), 2),
        );
        let _ = write!(
            svg.0,
            r#"<path d="M{:.2} {:.2} L{:.2} {:.2} L{x:.2} {:.2} Z" fill="{TEXT}"/>"#,
            x - 5.0,
            bar - 9.0,
            x + 5.0,
            bar - 9.0,
            bar - 3.0
        );
    }
    let mut y = bar + 56.0;
    for (index, tier) in tiers.iter().enumerate().rev() {
        let gap = tier["gap_gb"].as_f64();
        let ahead = gap.is_some_and(|gap| gap > 0.0);
        let color = if ahead { TEXT } else { MUTED };
        let figure_text = match gap {
            Some(gap) if gap > 0.0 => format!("{} à envoyer", size(Some(gap), 1)),
            Some(gap) => format!("{} de marge", size(Some(-gap), 1)),
            None => String::new(),
        };
        let name = format!(
            "{} {}",
            figure(tier["ratio"].as_f64(), 1),
            tier_name(tier["name"].as_str().unwrap_or_default())
        );
        svg.rect(
            PAD + 12.0,
            y - 8.0,
            8.0,
            8.0,
            2.0,
            TIER_COLORS[(index + 1) % 5],
        );
        svg.text(PAD + 28.0, y, 12.0, 400, color, "start", &name);
        svg.text(PAD + width - 12.0, y, 12.0, 600, color, "end", &figure_text);
        y += 18.0;
    }
    top + height
}

struct Plot {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Plot {
    fn x_of(&self, offset: i64) -> f64 {
        self.x + (offset as f64 + 0.5) * self.w / HISTORY_DAYS as f64
    }

    fn y_of(&self, value: f64, top: f64) -> f64 {
        self.y + self.h - 0.9 * self.h * value / top
    }
}

fn top_of(values: &[Option<f64>]) -> f64 {
    let found = values.iter().flatten().fold(0.0_f64, |a, b| a.max(*b));
    if found > 0.0 { found } else { 1.0 }
}

fn lines(svg: &mut Svg, plot: &Plot, color: &str, points: &[(i64, Option<f64>)], top: f64) {
    let mut runs: Vec<Vec<(i64, f64)>> = vec![];
    for (offset, value) in points {
        match value {
            Some(value) => match runs.last_mut() {
                Some(run) if run.last().is_some_and(|(at, _)| *at + 1 == *offset) => {
                    run.push((*offset, *value));
                }
                _ => runs.push(vec![(*offset, *value)]),
            },
            None => runs.push(vec![]),
        }
    }
    for run in runs.iter().filter(|run| !run.is_empty()) {
        if run.len() == 1 {
            let (x, y) = (plot.x_of(run[0].0), plot.y_of(run[0].1, top));
            let _ = write!(
                svg.0,
                r#"<circle cx="{x:.2}" cy="{y:.2}" r="3" fill="{color}"/>"#
            );
            continue;
        }
        let path: Vec<String> = run
            .iter()
            .map(|(at, value)| format!("{:.2},{:.2}", plot.x_of(*at), plot.y_of(*value, top)))
            .collect();
        let base = plot.y + plot.h;
        let _ = write!(
            svg.0,
            r#"<polygon points="{} {:.2},{base:.2} {:.2},{base:.2}" fill="{color}" fill-opacity="0.14"/><polyline points="{}" fill="none" stroke="{color}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>"#,
            path.join(" "),
            plot.x_of(run[run.len() - 1].0),
            plot.x_of(run[0].0),
            path.join(" ")
        );
    }
}

fn chart(
    svg: &mut Svg,
    frame: (f64, f64, f64, f64),
    caption: &str,
    keys: &[(&str, &str)],
    span: (Date, Date),
    draw: impl FnOnce(&mut Svg, &Plot),
) {
    let (x, y, w, h) = frame;
    svg.card(x, y, w, h);
    svg.label(x + 12.0, y + 22.0, caption);
    let mut key_x = x + 12.0 + 7.5 * caption.chars().count() as f64 + 10.0;
    for (color, text) in keys {
        svg.rect(key_x, y + 14.0, 8.0, 8.0, 2.0, color);
        svg.text(key_x + 12.0, y + 22.0, 11.0, 400, color, "start", text);
        key_x += 12.0 + 6.0 * text.chars().count() as f64 + 14.0;
    }
    let plot = Plot {
        x: x + 12.0,
        y: y + 36.0,
        w: w - 24.0,
        h: h - 66.0,
    };
    for step in 0..4 {
        let line = plot.y + plot.h * step as f64 / 4.0;
        let _ = write!(
            svg.0,
            r#"<line x1="{:.2}" y1="{line:.2}" x2="{:.2}" y2="{line:.2}" stroke="rgba(148,163,184,0.07)"/>"#,
            plot.x,
            plot.x + plot.w
        );
    }
    let bottom = plot.y + plot.h;
    let _ = write!(
        svg.0,
        r#"<line x1="{:.2}" y1="{bottom:.2}" x2="{:.2}" y2="{bottom:.2}" stroke="{LINE}"/>"#,
        plot.x,
        plot.x + plot.w
    );
    draw(svg, &plot);
    let day = |date: Date| date.strftime("%d/%m").to_string();
    svg.text(
        plot.x,
        bottom + 16.0,
        10.0,
        400,
        MUTED,
        "start",
        &day(span.0),
    );
    svg.text(
        plot.x + plot.w,
        bottom + 16.0,
        10.0,
        400,
        MUTED,
        "end",
        &day(span.1),
    );
}

fn trends(svg: &mut Svg, board: &Value, top: f64) -> f64 {
    let history = board["history"].as_array().cloned().unwrap_or_default();
    let dated: Vec<(Date, &Value)> = history
        .iter()
        .filter_map(|entry| Some((entry["day"].as_str()?.parse::<Date>().ok()?, entry)))
        .collect();
    let Some(end) = dated.last().map(|(day, _)| *day) else {
        return top - GAP;
    };
    let start = end
        .checked_sub(jiff::Span::new().days(HISTORY_DAYS - 1))
        .unwrap_or(end);
    let window: Vec<(i64, &Value)> = dated
        .iter()
        .filter(|(day, _)| *day >= start)
        .filter_map(|(day, entry)| Some((i64::from(start.until(*day).ok()?.get_days()), *entry)))
        .collect();
    let series = |key: &str| -> Vec<(i64, Option<f64>)> {
        window
            .iter()
            .map(|(at, entry)| (*at, entry[key].as_f64()))
            .collect()
    };
    let values = |points: &[(i64, Option<f64>)]| points.iter().map(|(_, v)| *v).collect::<Vec<_>>();
    let width = (WIDTH - 2.0 * PAD - GAP) / 2.0;
    let height = 210.0;
    let paid = series("zen_paid");
    let balance = series("zen");
    let (paid_top, balance_top) = (top_of(&values(&paid)), top_of(&values(&balance)));
    chart(
        svg,
        (PAD, top, width, height),
        "Zen",
        &[("#fbbf24", "payé chaque nuit"), ("#34d399", "solde")],
        (start, end),
        |svg, plot| {
            let bar = plot.w / HISTORY_DAYS as f64 * 0.6;
            for (at, value) in &paid {
                if let Some(value) = value.filter(|value| *value > 0.0) {
                    let y = plot.y_of(value, paid_top);
                    svg.rect(
                        plot.x_of(*at) - bar / 2.0,
                        y,
                        bar,
                        plot.y + plot.h - y,
                        0.0,
                        "rgba(251,191,36,0.85)",
                    );
                }
            }
            lines(svg, plot, "#34d399", &balance, balance_top);
            svg.text(
                plot.x,
                plot.y + 0.1 * plot.h - 3.0,
                10.0,
                400,
                MUTED,
                "start",
                &figure(Some(paid_top), 0),
            );
            svg.text(
                plot.x + plot.w,
                plot.y + 0.1 * plot.h - 3.0,
                10.0,
                400,
                MUTED,
                "end",
                &figure(Some(balance_top), 0),
            );
        },
    );
    let up = series("uploaded_gb");
    let down = series("downloaded_gb");
    let both = top_of(&[values(&up), values(&down)].concat());
    chart(
        svg,
        (PAD + width + GAP, top, width, height),
        "Envoi et réception",
        &[("#60a5fa", "Envoyé"), ("#fb7185", "Reçu")],
        (start, end),
        |svg, plot| {
            lines(svg, plot, "#60a5fa", &up, both);
            lines(svg, plot, "#fb7185", &down, both);
            svg.text(
                plot.x,
                plot.y + 0.1 * plot.h - 3.0,
                10.0,
                400,
                MUTED,
                "start",
                &size(Some(both), 0),
            );
        },
    );
    top + height
}

pub fn svg_of(board: &Value) -> String {
    let mut body = Svg(String::new());
    let mut y = kpis(&mut body, board, PAD) + GAP;
    y = gauge(&mut body, board, y) + GAP;
    y = trends(&mut body, board, y) + PAD;
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{y:.0}" viewBox="0 0 {WIDTH} {y:.0}" font-family="IBM Plex Sans"><defs><radialGradient id="glow" cx="0.2" cy="0" r="1" gradientUnits="objectBoundingBox"><stop offset="0" stop-color="#2e1065"/><stop offset="0.6" stop-color="#0f172a"/></radialGradient></defs><rect width="100%" height="100%" fill="#0f172a"/><rect width="100%" height="100%" fill="url(#glow)"/>{}</svg>"##,
        body.0
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn board() -> Value {
        json!({
            "updated_at": "2026-11-22T10:00:00+00:00",
            "account": {"ratio": 1.53, "floor": 2.0, "uploaded_gb": 1530.2, "downloaded_gb": 1000.0, "zen": 12345, "available_gb": 220.4, "paused": false},
            "tiers": [
                {"name": "blocked", "ratio": 0.6, "gap_gb": -800.0},
                {"name": "even", "ratio": 1.0, "gap_gb": -530.0},
                {"name": "floor", "ratio": 2.0, "gap_gb": 470.0},
                {"name": "goal", "ratio": 2.0, "gap_gb": 470.0}
            ],
            "history": [
                {"day": "2026-11-20", "uploaded_gb": 1500.0, "downloaded_gb": 990.0, "zen": 12000, "zen_paid": 300},
                {"day": "2026-11-21", "uploaded_gb": 1520.0, "downloaded_gb": 995.0, "zen": 12200, "zen_paid": 200},
                {"day": "2026-11-22", "uploaded_gb": 1530.2, "downloaded_gb": 1000.0, "zen": 12345, "uploaded_day_gb": 10.2, "downloaded_day_gb": 5.0}
            ],
            "seeds": 117
        })
    }

    fn now() -> ZonedDateTime {
        "2026-11-22T11:00:00+01:00[Europe/Paris]".parse().unwrap()
    }

    #[test]
    fn the_board_renders_into_a_png_with_its_figures() {
        let svg = svg_of(&board());
        assert!(svg.contains(">1,53<"));
        assert!(svg.contains(">1 530,2 Go<"));
        assert!(svg.contains(">470,0 Go à envoyer<"));
        assert!(svg.contains(">24/10<"));
        assert!(svg.contains(">22/11<"));
        let png = png_of(&svg).unwrap();
        assert_eq!(b"\x89PNG", &png[..4]);
        assert_eq!(1800, u32::from_be_bytes(png[16..20].try_into().unwrap()));
    }

    #[test]
    fn a_figure_is_written_the_french_way() {
        assert_eq!("12 345", figure(Some(12345.4), 0));
        assert_eq!("-1 000,50", figure(Some(-1000.5), 2));
        assert_eq!("0,0", figure(Some(-0.01), 1));
        assert_eq!("-", figure(None, 2));
    }

    #[test]
    fn the_tier_counts_the_tiers_already_reached() {
        assert_eq!(Some((2, 2.0)), tier_of(&board()));
        let mut unknown = board();
        unknown["account"]["ratio"] = Value::Null;
        assert_eq!(None, tier_of(&unknown));
        let mut close = board();
        close["account"]["ratio"] = json!(1.999);
        close["tiers"][2]["gap_gb"] = json!(0.0);
        assert_eq!(Some((2, 2.0)), tier_of(&close));
    }

    #[test]
    fn only_a_move_of_the_ratio_crosses_a_tier() {
        assert!(crossed(Some((2, 2.0)), Some((3, 2.0))));
        assert!(!crossed(Some((2, 2.0)), Some((2, 2.0))));
        assert!(!crossed(Some((3, 1.5)), Some((2, 2.5))));
        assert!(!crossed(None, Some((2, 2.0))));
    }

    #[test]
    fn the_embed_time_has_whole_seconds() {
        let at: ZonedDateTime = "2026-11-22T11:00:00.123456789+01:00[Europe/Paris]"
            .parse()
            .unwrap();
        assert_eq!(
            "2026-11-22T10:00:00+00:00",
            payload(&board(), &at)["embeds"][0]["timestamp"]
        );
    }

    #[test]
    fn a_message_whose_id_cannot_be_saved_is_still_edited() {
        let folder = tempfile::tempdir().unwrap();
        let poster = Poster::new(
            "1",
            "token",
            folder.path().join("missing").join("farm-discord.json"),
        )
        .unwrap();
        let calls = RefCell::new(vec![]);
        let send = |method: &str, path: &str, _: Vec<u8>, _: &str| {
            calls.borrow_mut().push(format!("{method} {path}"));
            Ok((200, json!({"id": "m1"})))
        };
        assert!(
            poster
                .publish(&board(), &now(), &send)
                .unwrap()
                .contains("cannot save")
        );
        poster.publish(&board(), &now(), &send).unwrap();
        assert_eq!(
            vec!["POST ?wait=true", "PATCH /messages/m1"],
            *calls.borrow()
        );
    }

    #[test]
    fn a_board_without_a_run_or_a_day_is_not_posted() {
        assert!(ready(&board()));
        let mut waiting = board();
        waiting["updated_at"] = Value::Null;
        assert!(!ready(&waiting));
        let mut empty = board();
        empty["history"] = json!([]);
        assert!(!ready(&empty));
    }

    #[test]
    fn the_message_of_the_day_is_edited_and_a_new_day_gets_a_new_one() {
        let folder = tempfile::tempdir().unwrap();
        let poster = Poster::new("1", "token", folder.path().join("farm-discord.json")).unwrap();
        let calls = RefCell::new(vec![]);
        let send = |method: &str, path: &str, body: Vec<u8>, kind: &str| {
            assert!(kind.starts_with("multipart/form-data; boundary="));
            assert!(String::from_utf8_lossy(&body).contains("attachment://farm.png"));
            calls.borrow_mut().push(format!("{method} {path}"));
            Ok(match method {
                "POST" => (200, json!({"id": "m1"})),
                _ => (200, json!({})),
            })
        };
        assert_eq!(
            "Farm board posted, message m1 of 2026-11-22",
            poster.publish(&board(), &now(), &send).unwrap()
        );
        assert_eq!(
            "Farm board edited, message m1 of 2026-11-22",
            poster.publish(&board(), &now(), &send).unwrap()
        );
        let tomorrow: ZonedDateTime = "2026-11-23T00:30:00+01:00[Europe/Paris]".parse().unwrap();
        poster.publish(&board(), &tomorrow, &send).unwrap();
        assert_eq!(
            vec!["POST ?wait=true", "PATCH /messages/m1", "POST ?wait=true"],
            *calls.borrow()
        );
    }

    #[test]
    fn a_deleted_message_is_posted_again() {
        let folder = tempfile::tempdir().unwrap();
        let state = folder.path().join("farm-discord.json");
        fs::write(&state, r#"{"day": "2026-11-22", "message_id": "gone"}"#).unwrap();
        let poster = Poster::new("1", "token", state.clone()).unwrap();
        let send = |method: &str, _: &str, _: Vec<u8>, _: &str| {
            Ok(match method {
                "PATCH" => (404, json!({"message": "Unknown Message"})),
                _ => (200, json!({"id": "m2"})),
            })
        };
        poster.publish(&board(), &now(), &send).unwrap();
        let saved: Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
        assert_eq!("m2", saved["message_id"]);
    }

    #[test]
    fn the_next_post_waits_for_the_next_hour() {
        let at: ZonedDateTime = "2026-11-22T10:59:50+01:00[Europe/Paris]".parse().unwrap();
        assert_eq!(40, until_next_hour(&at).as_secs());
        assert_eq!(3630, until_next_hour(&now()).as_secs());
    }

    #[test]
    fn no_webhook_means_no_poster() {
        assert!(Poster::new("", "token", PathBuf::from("x")).is_none());
        assert!(Poster::new("1", " ", PathBuf::from("x")).is_none());
    }
}
