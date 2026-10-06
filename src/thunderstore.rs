// Custom maps from Thunderstore: "map_mods" (or --map-mods, which a hosting panel fills) lists
// packages by their Thunderstore link. At startup each one is brought to its latest version (or
// the version the link names) and only its reskate-levels.json is kept, in Mods/<owner>-<name>/
// next to a .thunderstore.json that says where it came from. The server never needs the map
// itself, so only that file is read out of the package's zip: with HTTP range requests a few
// kilobytes are fetched, not the whole map (a host that cannot do ranges gets the zip downloaded
// to a temporary file, deleted right after). A package that is no longer listed has its folder
// removed; folders without .thunderstore.json are left alone. While Thunderstore cannot be
// reached, what is already there is kept.
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::time::Duration;

pub const MARKER: &str = ".thunderstore.json";
const MANIFEST: &str = "reskate-levels.json";
const API: &str = "https://thunderstore.io/api/experimental/package";
const DOWNLOAD: &str = "https://thunderstore.io/package/download";
// The zip's end record and comment fit in this; ranges are fetched in pieces at least this large.
const TAIL: u64 = 65_557;
const MAX_DIRECTORY: u64 = 64 * 1024 * 1024;
const MAX_MANIFEST: u64 = 1024 * 1024;
// The largest zip downloaded whole when the host cannot send ranges.
const MAX_ZIP: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Package {
    pub namespace: String,
    pub name: String,
    pub version: Option<String>, // none: the latest
}

impl Package {
    pub fn folder(&self) -> String {
        format!("{}-{}", self.namespace, self.name)
    }
}

fn valid_part(part: &str) -> bool {
    !part.is_empty() && part.len() <= 128 && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.len() <= 9 && p.chars().all(|c| c.is_ascii_digit()))
}

// A Thunderstore link as the site shows it, https://thunderstore.io/c/<game>/p/<owner>/<name>/
// (with /v/<version>/ to pin a version), the older /package/<owner>/<name>/ form, a download
// link, or the package's id: Owner-Name or Owner-Name-1.2.3.
pub fn parse_package(text: &str) -> Option<Package> {
    let text = text.trim();
    let package = |namespace: &str, name: &str, version: Option<&str>| {
        (valid_part(namespace) && valid_part(name) && version.is_none_or(valid_version)).then(|| Package {
            namespace: namespace.into(),
            name: name.into(),
            version: version.map(str::to_string),
        })
    };
    if let Some(rest) = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")) {
        let (host, path) = rest.split_once('/')?;
        if host != "thunderstore.io" && !host.ends_with(".thunderstore.io") {
            return None;
        }
        let path = path.split(['?', '#']).next()?;
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        let at = |i: usize| parts.get(i).copied();
        let start = match (at(0), at(1), at(2)) {
            (Some("c"), _, Some("p")) => 3,
            (Some("package"), Some("download"), _) => 2,
            (Some("package"), _, _) => 1,
            _ => return None,
        };
        let (namespace, name) = (at(start)?, at(start + 1)?);
        let version = match at(start + 2) {
            None => None,
            Some("v") => Some(at(start + 3)?),
            Some(version) if valid_version(version) => Some(version),
            Some(_) => None,
        };
        return package(namespace, name, version);
    }
    // Owner-Name[-1.2.3]; owners and names hold no '-'.
    let parts: Vec<&str> = text.split('-').collect();
    match parts.as_slice() {
        [namespace, name] => package(namespace, name, None),
        [namespace, name, version] => package(namespace, name, Some(version)),
        _ => None,
    }
}

pub(crate) fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(30))
        .redirects(5)
        .user_agent(&format!("ReSkateServer/{} (Linux)", crate::update::VERSION))
        .build()
}

fn http_error(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(status, response) => format!("{} answered HTTP {status}", host(response.get_url())),
        ureq::Error::Transport(t) => format!("Thunderstore could not be reached ({})", t.kind()),
    }
}

fn host(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url)
}

fn latest_version(agent: &ureq::Agent, package: &Package) -> Result<String, String> {
    let url = format!("{API}/{}/{}/", package.namespace, package.name);
    let response = agent.get(&url).call().map_err(|e| match e {
        ureq::Error::Status(404, _) => "Thunderstore has no such package".to_string(),
        e => http_error(e),
    })?;
    let mut text = String::new();
    response.into_reader().take(MAX_MANIFEST).read_to_string(&mut text).map_err(|e| e.to_string())?;
    let root: Value = serde_json::from_str(&text).map_err(|_| "Thunderstore's answer is not JSON".to_string())?;
    let version = root.pointer("/latest/version_number").and_then(Value::as_str).unwrap_or_default();
    valid_version(version).then(|| version.to_string()).ok_or_else(|| "Thunderstore's answer names no version".into())
}

// Where a zip's bytes come from.
trait Source {
    fn len(&self) -> u64;
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, String>;
}

struct FileSource(File, u64);

impl Source for FileSource {
    fn len(&self) -> u64 {
        self.1
    }
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let end = offset.checked_add(len).filter(|&e| e <= self.1).ok_or("the zip is cut off")?;
        let mut bytes = vec![0; (end - offset) as usize];
        self.0.read_exact_at(&mut bytes, offset).map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

impl Source for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let end = offset.checked_add(len).filter(|&e| e <= self.as_slice().len() as u64).ok_or("the zip is cut off")?;
        Ok(self[offset as usize..end as usize].to_vec())
    }
}

// Ranges of the zip, fetched as they are read; the last piece is kept, as the zip's records are
// read in small steps.
struct HttpSource {
    agent: ureq::Agent,
    url: String,
    len: u64,
    cached: (u64, Vec<u8>),
}

impl HttpSource {
    fn get(&self, from: u64, to: u64) -> Result<Vec<u8>, String> {
        let response = self.agent.get(&self.url).set("Range", &format!("bytes={from}-{}", to - 1)).call().map_err(http_error)?;
        if response.status() != 206 {
            return Err("the download host stopped sending ranges".into());
        }
        let mut bytes = Vec::new();
        response.into_reader().take(to - from).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != to - from {
            return Err("the download was cut off".into());
        }
        Ok(bytes)
    }
}

impl Source for HttpSource {
    fn len(&self) -> u64 {
        self.len
    }
    fn read_at(&mut self, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let end = offset.checked_add(len).filter(|&e| e <= self.len).ok_or("the zip is cut off")?;
        let (start, bytes) = &self.cached;
        if offset < *start || end > start + bytes.len() as u64 {
            let to = end.max(offset.saturating_add(TAIL)).min(self.len);
            self.cached = (offset, self.get(offset, to)?);
        }
        let (start, bytes) = &self.cached;
        Ok(bytes[(offset - start) as usize..(end - start) as usize].to_vec())
    }
}

fn u16_at(b: &[u8], at: usize) -> u64 {
    u16::from_le_bytes([b[at], b[at + 1]]) as u64
}

fn u32_at(b: &[u8], at: usize) -> u64 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap()) as u64
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

// Every reskate-levels.json in the zip, wherever it lies in the package's folders.
fn read_manifests(zip: &mut dyn Source) -> Result<Vec<Vec<u8>>, String> {
    let len = zip.len();
    let tail_start = len.saturating_sub(TAIL);
    let tail = zip.read_at(tail_start, len - tail_start)?;
    let end = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("the download is not a zip")?;
    let (mut entries, mut size, mut offset) = (u16_at(&tail, end + 10), u32_at(&tail, end + 12), u32_at(&tail, end + 16));
    if entries == 0xffff || size == 0xffff_ffff || offset == 0xffff_ffff {
        // Zip64: a locator just before the end record points at the larger one.
        let locator = (end >= 20).then(|| end - 20).filter(|&l| tail[l..l + 4] == [0x50, 0x4b, 0x06, 0x07]).ok_or("the zip64 record is missing")?;
        let record = zip.read_at(u64_at(&tail, locator + 8), 56)?;
        if record[..4] != [0x50, 0x4b, 0x06, 0x06] {
            return Err("the zip64 record is damaged".into());
        }
        (entries, size, offset) = (u64_at(&record, 32), u64_at(&record, 40), u64_at(&record, 48));
    }
    if size > MAX_DIRECTORY {
        return Err("the zip's file list is too large".into());
    }
    let directory = zip.read_at(offset, size)?;
    let mut found = Vec::new();
    let mut at = 0;
    for _ in 0..entries {
        if at + 46 > directory.len() || directory[at..at + 4] != [0x50, 0x4b, 0x01, 0x02] {
            return Err("the zip's file list is damaged".into());
        }
        let method = u16_at(&directory, at + 10);
        let (mut packed, mut unpacked) = (u32_at(&directory, at + 20), u32_at(&directory, at + 24));
        let (name_len, extra_len, comment_len) =
            (u16_at(&directory, at + 28) as usize, u16_at(&directory, at + 30) as usize, u16_at(&directory, at + 32) as usize);
        let mut local = u32_at(&directory, at + 42);
        let next = at + 46 + name_len + extra_len + comment_len;
        if next > directory.len() {
            return Err("the zip's file list is damaged".into());
        }
        let name = String::from_utf8_lossy(&directory[at + 46..at + 46 + name_len]).into_owned();
        // Zip64 sizes and offset, for the fields that overflowed, in this order.
        let mut extra = &directory[at + 46 + name_len..at + 46 + name_len + extra_len];
        while extra.len() >= 4 {
            let (id, data_len) = (u16_at(extra, 0), u16_at(extra, 2) as usize);
            let data = &extra[4..(4 + data_len).min(extra.len())];
            if id == 1 {
                let mut i = 0;
                for field in [&mut unpacked, &mut packed, &mut local] {
                    if *field == 0xffff_ffff && i + 8 <= data.len() {
                        *field = u64_at(data, i);
                        i += 8;
                    }
                }
            }
            extra = &extra[(4 + data_len).min(extra.len())..];
        }
        at = next;
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or_default();
        if !file_name.eq_ignore_ascii_case(MANIFEST) {
            continue;
        }
        if unpacked > MAX_MANIFEST || packed > MAX_MANIFEST {
            return Err(format!("{name} is too large"));
        }
        let header = zip.read_at(local, 30)?;
        if header[..4] != [0x50, 0x4b, 0x03, 0x04] {
            return Err("the zip is damaged".into());
        }
        let data = zip.read_at(local + 30 + u16_at(&header, 26) + u16_at(&header, 28), packed)?;
        let bytes = match method {
            0 => data,
            8 => {
                let mut out = Vec::new();
                flate2::read::DeflateDecoder::new(&data[..]).take(MAX_MANIFEST).read_to_end(&mut out).map_err(|_| format!("{name} is damaged"))?;
                out
            }
            _ => return Err(format!("{name} is packed in a way the server cannot read")),
        };
        found.push(bytes);
    }
    Ok(found)
}

// The package's reskate-levels.json files merged into one: {"levels":[...]}.
fn merge_manifests(files: Vec<Vec<u8>>) -> Result<Value, String> {
    if files.is_empty() {
        return Err(format!("the package has no {MANIFEST}, so it is not a ReSkate map"));
    }
    let mut levels = Vec::new();
    for bytes in files {
        let text = String::from_utf8_lossy(&bytes);
        let root: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("its {MANIFEST} is not JSON: {e}"))?;
        let list = root.get("levels").and_then(Value::as_array).ok_or(format!("its {MANIFEST} lists no levels"))?;
        levels.extend(list.iter().cloned());
    }
    Ok(json!({ "levels": levels }))
}

fn download_manifest(agent: &ureq::Agent, package: &Package, version: &str, temp: &Path) -> Result<Value, String> {
    manifest_from(agent, &format!("{DOWNLOAD}/{}/{}/{version}/", package.namespace, package.name), temp)
}

pub(crate) fn manifest_from(agent: &ureq::Agent, url: &str, temp: &Path) -> Result<Value, String> {
    // The last bytes first: the zip's end record, and whether the host can send ranges at all.
    let response = agent.get(url).set("Range", &format!("bytes=-{TAIL}")).call().map_err(http_error)?;
    let final_url = response.get_url().to_string();
    if response.status() == 206 {
        let total = response
            .header("Content-Range")
            .and_then(|r| r.rsplit('/').next())
            .and_then(|t| t.trim().parse::<u64>().ok())
            .ok_or("the download host sent no length")?;
        let mut bytes = Vec::new();
        response.into_reader().take(TAIL).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        let start = total.checked_sub(bytes.len() as u64).ok_or("the download host sent a bad range")?;
        let mut source = HttpSource { agent: agent.clone(), url: final_url, len: total, cached: (start, bytes) };
        return merge_manifests(read_manifests(&mut source)?);
    }
    // No ranges: the whole zip, to a file that goes again right after.
    let result = (|| {
        let mut file = File::create(temp).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut response.into_reader().take(MAX_ZIP + 1), &mut file).map_err(|e| format!("the download was cut off: {e}"))?;
        if copied > MAX_ZIP {
            return Err("the package is larger than 4 GB".to_string());
        }
        file.flush().map_err(|e| e.to_string())?;
        let mut source = FileSource(File::open(temp).map_err(|e| e.to_string())?, copied);
        merge_manifests(read_manifests(&mut source)?)
    })();
    let _ = std::fs::remove_file(temp);
    result
}

fn installed_version(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join(MARKER)).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    root.get("version")?.as_str().map(str::to_string)
}

pub struct SyncReport {
    pub lines: Vec<String>,
    // The assets of maps that came with packages installed for the first time.
    pub new_maps: Vec<String>,
}

// Brings Mods/ in line with `links`; the lines say what happened, for the log.
pub fn sync(mods: &Path, links: &[String]) -> SyncReport {
    let mut report = SyncReport { lines: Vec::new(), new_maps: Vec::new() };
    let mut wanted = Vec::new();
    for link in links.iter().filter(|l| !l.trim().is_empty()) {
        match parse_package(link) {
            Some(package) if !wanted.iter().any(|p: &Package| p.folder().eq_ignore_ascii_case(&package.folder())) => wanted.push(package),
            Some(_) => {}
            None => report.lines.push(format!("Map mods: \"{link}\" is not a Thunderstore package link.")),
        }
    }
    // Packages taken off the list.
    if let Ok(entries) = std::fs::read_dir(mods) {
        for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if path.join(MARKER).is_file() && !wanted.iter().any(|p| p.folder().eq_ignore_ascii_case(&name)) {
                match std::fs::remove_dir_all(&path) {
                    Ok(()) => report.lines.push(format!("Map mods: removed {name}, which is no longer listed.")),
                    Err(e) => report.lines.push(format!("Map mods: could not remove {name}: {e}")),
                }
            }
        }
    }
    if wanted.is_empty() {
        return report;
    }
    if let Err(e) = std::fs::create_dir_all(mods) {
        report.lines.push(format!("Map mods: cannot create {}: {e}", mods.display()));
        return report;
    }
    let agent = agent();
    for package in wanted {
        let folder = mods.join(package.folder());
        let installed = installed_version(&folder);
        let result = (|| -> Result<Option<String>, String> {
            let version = match &package.version {
                Some(version) => version.clone(),
                None => latest_version(&agent, &package)?,
            };
            if installed.as_deref() == Some(version.as_str()) && folder.join(MANIFEST).is_file() {
                return Ok(None);
            }
            let manifest = download_manifest(&agent, &package, &version, &mods.join(format!(".{}.zip.part", package.folder())))?;
            std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
            let text = serde_json::to_string_pretty(&manifest).unwrap_or_default();
            std::fs::write(folder.join(MANIFEST), text + "\n").map_err(|e| e.to_string())?;
            let marker = json!({ "package": format!("{}-{}", package.namespace, package.name), "version": version });
            std::fs::write(folder.join(MARKER), serde_json::to_string_pretty(&marker).unwrap_or_default() + "\n").map_err(|e| e.to_string())?;
            if installed.is_none() {
                let levels = manifest["levels"].as_array().into_iter().flatten();
                report.new_maps.extend(levels.filter_map(|l| l.get("asset")?.as_str()).map(str::to_string));
            }
            Ok(Some(version))
        })();
        let name = package.folder();
        match (result, &installed) {
            (Ok(None), _) => {}
            (Ok(Some(version)), None) => report.lines.push(format!("Map mods: installed {name} {version}.")),
            (Ok(Some(version)), Some(old)) => report.lines.push(format!("Map mods: updated {name} from {old} to {version}.")),
            (Err(e), Some(old)) => report.lines.push(format!("Map mods: kept {name} {old}; could not check for a newer version: {e}.")),
            (Err(e), None) => report.lines.push(format!("Map mods: could not install {name}: {e}.")),
        }
    }
    report
}

// Links as a panel variable holds them: separated by commas, semicolons or whitespace.
pub fn split_links(text: &str) -> Vec<String> {
    text.split([',', ';', ' ', '\n', '\r', '\t']).filter(|l| !l.is_empty()).map(str::to_string).collect()
}

// For the tests: the merged manifest of a zip held in memory.
#[cfg(test)]
pub fn manifests_in_zip(mut zip: Vec<u8>) -> Result<Value, String> {
    merge_manifests(read_manifests(&mut zip)?)
}
