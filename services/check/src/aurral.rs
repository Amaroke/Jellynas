use crate::api::{Api, fetch};
use crate::input::Input;
use crate::report::{Report, Severity, trimmed};
use serde_json::{Value, json};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const LIMIT: usize = 200;

#[derive(Debug, PartialEq)]
pub enum Problem {
    Id3BeforeFlac,
    PictureWithoutType,
    NotFlac,
    Truncated,
}

impl Problem {
    fn name(&self) -> &'static str {
        match self {
            Problem::Id3BeforeFlac => "ID3 tag before the FLAC stream",
            Problem::PictureWithoutType => "embedded picture without a type",
            Problem::NotFlac => "not a FLAC file",
            Problem::Truncated => "truncated metadata",
        }
    }

    fn repairable(&self) -> bool {
        matches!(self, Problem::Id3BeforeFlac | Problem::PictureWithoutType)
    }
}

struct Block {
    kind: u8,
    body: Vec<u8>,
}

struct Flac {
    blocks: Vec<Block>,
    audio: usize,
}

fn id3_length(data: &[u8]) -> Option<usize> {
    if data.len() < 10 || &data[..3] != b"ID3" {
        return None;
    }
    let size = data[6..10]
        .iter()
        .fold(0usize, |size, byte| (size << 7) | usize::from(byte & 0x7f));
    let footer = if data[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + size + footer)
}

fn parse(data: &[u8], start: usize) -> Result<Flac, Problem> {
    if data.get(start..start + 4) != Some(b"fLaC") {
        return Err(Problem::NotFlac);
    }
    let mut position = start + 4;
    let mut blocks = vec![];
    loop {
        let header = data.get(position..position + 4).ok_or(Problem::Truncated)?;
        let last = header[0] & 0x80 != 0;
        let length =
            usize::from(header[1]) << 16 | usize::from(header[2]) << 8 | usize::from(header[3]);
        let body = data
            .get(position + 4..position + 4 + length)
            .ok_or(Problem::Truncated)?;
        blocks.push(Block {
            kind: header[0] & 0x7f,
            body: body.to_vec(),
        });
        position += 4 + length;
        if last {
            return Ok(Flac {
                blocks,
                audio: position,
            });
        }
    }
}

fn word(body: &[u8], at: usize) -> Option<usize> {
    body.get(at..at + 4)
        .map(|bytes| u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize)
}

fn picture_type(body: &[u8]) -> Option<&'static [u8]> {
    let description = word(body, 8)?;
    let image = body.get(8 + 4 + description + 16 + 4..)?;
    if image.starts_with(&[0xff, 0xd8]) {
        Some(b"image/jpeg")
    } else if image.starts_with(b"\x89PNG") {
        Some(b"image/png")
    } else {
        None
    }
}

fn untyped(block: &Block) -> bool {
    block.kind == 6 && word(&block.body, 4) == Some(0)
}

#[cfg(test)]
fn problems(data: &[u8]) -> Vec<Problem> {
    let start = id3_length(data).unwrap_or(0);
    let flac = match parse(data, start) {
        Ok(flac) => flac,
        Err(problem) => return vec![problem],
    };
    let mut found = vec![];
    if start > 0 {
        found.push(Problem::Id3BeforeFlac);
    }
    if flac.blocks.iter().any(untyped) {
        found.push(Problem::PictureWithoutType);
    }
    found
}

pub fn repaired(data: &[u8]) -> Option<Vec<u8>> {
    let start = id3_length(data).unwrap_or(0);
    let flac = parse(data, start).ok()?;
    let mut changed = start > 0;
    let mut out = b"fLaC".to_vec();
    let count = flac.blocks.len();
    for (index, block) in flac.blocks.iter().enumerate() {
        let mut body = block.body.clone();
        if untyped(block) {
            let kind = picture_type(&body)?;
            let mut typed = body[..4].to_vec();
            typed.extend_from_slice(&(kind.len() as u32).to_be_bytes());
            typed.extend_from_slice(kind);
            typed.extend_from_slice(&body[8..]);
            body = typed;
            changed = true;
        }
        let last = if index + 1 == count { 0x80 } else { 0 };
        out.push(block.kind | last);
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&body);
    }
    out.extend_from_slice(&data[flac.audio..]);
    changed.then_some(out)
}

fn flacs(root: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => flacs(&path, found),
            Ok(kind)
                if kind.is_file()
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("flac")) =>
            {
                found.push(path)
            }
            _ => {}
        }
    }
}

pub fn scan(root: &Path) -> Vec<(PathBuf, Vec<Problem>)> {
    let mut paths = vec![];
    flacs(root, &mut paths);
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let found = file_problems(&path).ok()?;
            (!found.is_empty()).then_some((path, found))
        })
        .collect()
}

fn exact(file: &mut fs::File, size: usize) -> Option<Vec<u8>> {
    let mut buffer = vec![0; size];
    file.read_exact(&mut buffer).ok().map(|_| buffer)
}

fn file_problems(path: &Path) -> std::io::Result<Vec<Problem>> {
    let mut file = fs::File::open(path)?;
    let Some(head) = exact(&mut file, 10) else {
        return Ok(vec![Problem::NotFlac]);
    };
    let start = id3_length(&head).unwrap_or(0);
    file.seek(SeekFrom::Start(start as u64))?;
    if exact(&mut file, 4).as_deref() != Some(b"fLaC") {
        return Ok(vec![Problem::NotFlac]);
    }
    let mut found = vec![];
    if start > 0 {
        found.push(Problem::Id3BeforeFlac);
    }
    loop {
        let Some(header) = exact(&mut file, 4) else {
            return Ok(vec![Problem::Truncated]);
        };
        let length =
            usize::from(header[1]) << 16 | usize::from(header[2]) << 8 | usize::from(header[3]);
        let mut skipped = length;
        if header[0] & 0x7f == 6 && length >= 8 {
            let Some(start) = exact(&mut file, 8) else {
                return Ok(vec![Problem::Truncated]);
            };
            if word(&start, 4) == Some(0) && !found.contains(&Problem::PictureWithoutType) {
                found.push(Problem::PictureWithoutType);
            }
            skipped -= 8;
        }
        let next = file.stream_position()? + skipped as u64;
        if next > file.metadata()?.len() {
            return Ok(vec![Problem::Truncated]);
        }
        file.seek(SeekFrom::Start(next))?;
        if header[0] & 0x80 != 0 {
            return Ok(found);
        }
    }
}

pub fn repair(root: &Path) -> Result<Value, String> {
    let mut fixed = vec![];
    let mut left = vec![];
    for (path, found) in scan(root) {
        let shown = path.display().to_string();
        if !found.iter().all(Problem::repairable) {
            left.push(shown);
            continue;
        }
        let data = fs::read(&path).map_err(|error| format!("{shown}: {error}"))?;
        let Some(out) = repaired(&data) else {
            left.push(shown);
            continue;
        };
        let temporary = path.with_extension("flac.repair");
        fs::write(&temporary, &out).map_err(|error| format!("{shown}: {error}"))?;
        let permissions = fs::metadata(&path)
            .map_err(|error| format!("{shown}: {error}"))?
            .permissions();
        fs::set_permissions(&temporary, permissions)
            .map_err(|error| format!("{shown}: {error}"))?;
        fs::rename(&temporary, &path).map_err(|error| format!("{shown}: {error}"))?;
        fixed.push(shown);
    }
    Ok(json!({"repaired": fixed, "left": left}))
}

fn job(line: &str) -> Value {
    let fields: Vec<&str> = line.split('|').collect();
    let field = |index: usize| fields.get(index).copied().unwrap_or("");
    json!({
        "id": field(1),
        "artist": field(2),
        "track": field(3),
        "album": field(4),
        "format": field(5),
        "error": field(6),
    })
}

pub fn check(report: &mut Report, api: &impl Api, input: &Input, library: &Path) {
    if let Some(status) = fetch(report, api, "/playlists/status") {
        let worker = &status["worker"]["settings"];
        if worker["paused"] == json!(true) {
            report.flag(
                Severity::High,
                "aurral",
                "The download worker of Aurral is paused",
            );
        }
        if status["slskd"]["configured"] != json!(true) {
            report.flag(
                Severity::High,
                "aurral",
                "slskd is not configured in Aurral",
            );
        }
        report.fact(
            "status",
            json!({
                "worker": worker,
                "stats": status["staticStats"],
                "reviewCount": status["reviewCount"],
            }),
        );
    }
    let rows = |state: &str| -> Vec<Value> {
        input
            .jobs
            .iter()
            .filter(|line| line.split('|').next() == Some(state))
            .map(|line| job(line))
            .collect()
    };
    let failed = rows("failed");
    let lossy = rows("lossy");
    if !failed.is_empty() {
        report.flag(
            Severity::Medium,
            "aurral",
            format!("{} tracks failed to download", failed.len()),
        );
    }
    report.fact("failed", trimmed(&json!(failed), LIMIT));
    report.fact("lossy", trimmed(&json!(lossy), LIMIT));
    let broken: Vec<Value> = scan(library)
        .into_iter()
        .map(|(path, found)| {
            json!({
                "path": path.display().to_string(),
                "problems": found.iter().map(Problem::name).collect::<Vec<_>>(),
                "repairable": found.iter().all(Problem::repairable),
            })
        })
        .collect();
    if !broken.is_empty() {
        report.flag(
            Severity::High,
            "aurral",
            format!(
                "{} files of the Library cannot play in a browser",
                broken.len()
            ),
        );
    }
    report.fact("broken", json!(broken));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::fake::Fake;

    fn block(kind: u8, last: bool, body: &[u8]) -> Vec<u8> {
        let mut out = vec![kind | if last { 0x80 } else { 0 }];
        out.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(body);
        out
    }

    fn picture(kind: &[u8], image: &[u8]) -> Vec<u8> {
        let mut body = 3u32.to_be_bytes().to_vec();
        body.extend_from_slice(&(kind.len() as u32).to_be_bytes());
        body.extend_from_slice(kind);
        body.extend_from_slice(&0u32.to_be_bytes());
        body.extend_from_slice(&[0; 16]);
        body.extend_from_slice(&(image.len() as u32).to_be_bytes());
        body.extend_from_slice(image);
        body
    }

    fn flac(picture_kind: &[u8]) -> Vec<u8> {
        let mut out = b"fLaC".to_vec();
        out.extend(block(0, false, &[1; 34]));
        out.extend(block(
            6,
            true,
            &picture(picture_kind, &[0xff, 0xd8, 0xff, 0xe0]),
        ));
        out.extend_from_slice(b"AUDIO");
        out
    }

    fn id3(inner: &[u8]) -> Vec<u8> {
        let mut out = b"ID3\x04\x00\x00\x00\x00\x00\x03TAG".to_vec();
        out.extend_from_slice(inner);
        out
    }

    #[test]
    fn a_clean_file_has_no_problem() {
        assert!(problems(&flac(b"image/jpeg")).is_empty());
        assert_eq!(None, repaired(&flac(b"image/jpeg")));
    }

    #[test]
    fn a_picture_without_type_gets_the_type_of_its_image() {
        let broken = flac(b"");
        assert_eq!(vec![Problem::PictureWithoutType], problems(&broken));
        assert_eq!(Some(flac(b"image/jpeg")), repaired(&broken));
    }

    #[test]
    fn an_id3_tag_before_the_stream_is_cut() {
        let broken = id3(&flac(b"image/jpeg"));
        assert_eq!(vec![Problem::Id3BeforeFlac], problems(&broken));
        assert_eq!(Some(flac(b"image/jpeg")), repaired(&broken));
    }

    #[test]
    fn both_problems_are_repaired_at_once() {
        let broken = id3(&flac(b""));
        assert_eq!(Some(flac(b"image/jpeg")), repaired(&broken));
    }

    #[test]
    fn another_format_is_reported_and_left_alone() {
        let mp3 = id3(b"\xff\xfbAUDIO");
        assert_eq!(vec![Problem::NotFlac], problems(&mp3));
        assert_eq!(None, repaired(&mp3));
    }

    #[test]
    fn cut_metadata_is_truncated() {
        let mut short = b"fLaC".to_vec();
        short.extend_from_slice(&[0x80, 0, 0, 34, 1, 2]);
        assert_eq!(vec![Problem::Truncated], problems(&short));
    }

    #[test]
    fn repair_rewrites_only_the_broken_files() {
        let root = std::env::temp_dir().join(format!("check-aurral-{}", std::process::id()));
        let album = root.join("Artist/Album");
        fs::create_dir_all(&album).unwrap();
        fs::write(album.join("01.flac"), flac(b"")).unwrap();
        fs::write(album.join("02.flac"), flac(b"image/jpeg")).unwrap();
        fs::write(album.join("03.flac"), id3(b"\xff\xfb")).unwrap();
        let out = repair(&root).unwrap();
        assert_eq!(1, out["repaired"].as_array().unwrap().len());
        assert_eq!(1, out["left"].as_array().unwrap().len());
        assert_eq!(
            flac(b"image/jpeg"),
            fs::read(album.join("01.flac")).unwrap()
        );
        assert_eq!(1, scan(&root).len());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn reading_a_file_finds_what_parsing_it_finds() {
        let root = std::env::temp_dir().join(format!("check-aurral-read-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let mut cut = flac(b"");
        cut.truncate(20);
        let cases = [
            flac(b"image/jpeg"),
            flac(b""),
            id3(&flac(b"")),
            id3(b"\xff\xfbAUDIO"),
            cut,
        ];
        for (index, data) in cases.iter().enumerate() {
            let path = root.join(format!("{index}.flac"));
            fs::write(&path, data).unwrap();
            assert_eq!(
                problems(data),
                file_problems(&path).unwrap(),
                "case {index}"
            );
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn failed_tracks_and_broken_files_are_flagged() {
        let root = std::env::temp_dir().join(format!("check-aurral-report-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a.flac"), flac(b"")).unwrap();
        let api = Fake::new(&[(
            "/playlists/status",
            json!({"worker": {"settings": {"paused": false}}, "slskd": {"configured": true}, "staticStats": {"failed": 1}}),
        )]);
        let input = crate::input::parse(
            "job|aurral|failed|j1|Artist|Song|Album||no match\njob|aurral|lossy|j2|Artist|Other|Album|mp3|\n",
        );
        let mut report = Report::default();
        check(&mut report, &api, &input, &root);
        assert!(report.flagged(Severity::Medium, "1 tracks failed"));
        assert!(report.flagged(Severity::High, "1 files of the Library"));
        assert_eq!(report.facts["failed"]["items"][0]["track"], "Song");
        assert_eq!(report.facts["lossy"]["items"][0]["format"], "mp3");
        fs::remove_dir_all(&root).unwrap();
    }
}
