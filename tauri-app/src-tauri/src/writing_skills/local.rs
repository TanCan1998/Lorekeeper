//! 本机 Skill 包；主进程持有路径，安装和生成共享清单验证。
use super::{
    inspect_skill, inspect_writing_skill_markdown, parse_frontmatter_mode, WritingSkillInspection,
    MAX_SKILL_BYTES,
};
use crate::services::portable_project_archive::{handle_identity, link_count, path_identity};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
pub const BUNDLE_MANIFEST: &str = ".lorekeeper-bundle.json";
fn failure() -> String {
    "无法安全读取 Skill / Cannot safely read skill".into()
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BundledFile {
    pub path: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum InstalledBundle {
    None,
    Intact { files: Vec<BundledFile> },
    Damaged,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInspection {
    #[serde(flatten)]
    pub skill: WritingSkillInspection,
    pub bundled_files: Vec<String>,
    pub unbundled_references: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct LoadedPackage {
    pub raw: String,
    pub inspection: PackageInspection,
    pub files: Vec<BundledFile>,
    pub skipped_files: Vec<String>,
    pub source_name: String,
    pub content_sha256: String,
}
fn text_extension(path: &str) -> bool {
    [".md", ".markdown", ".txt"]
        .iter()
        .any(|ext| path.to_ascii_lowercase().ends_with(ext))
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|p| {
            !p.is_empty()
                && p != "."
                && p != ".."
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
        })
}
fn safe_root(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err(failure());
    }
    for p in root.ancestors() {
        let i = path_identity(p).map_err(|_| failure())?;
        if i.is_symlink || !i.is_dir {
            return Err(failure());
        }
    }
    Ok(())
}
/// 有界读取并复核路径/句柄身份，拒绝链接和共享硬链接。
fn read_selected(path: &Path) -> Result<Vec<u8>, String> {
    safe_root(path.parent().ok_or_else(failure)?)?;
    let before = path_identity(path).map_err(|_| failure())?;
    if before.is_symlink
        || !before.is_file
        || before.size > MAX_SKILL_BYTES as u64
        || link_count(path)? != 1
    {
        return Err(failure());
    }
    let mut f = File::open(path).map_err(|_| failure())?;
    if handle_identity(&f)? != before {
        return Err(failure());
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(MAX_SKILL_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| failure())?;
    if bytes.len() > MAX_SKILL_BYTES
        || bytes.contains(&0)
        || handle_identity(&f)? != before
        || path_identity(path)? != before
        || link_count(path)? != 1
    {
        return Err(failure());
    }
    Ok(bytes)
}
pub fn read_file(root: &Path, relative: &str) -> Result<Option<Vec<u8>>, String> {
    safe_root(root)?;
    if !safe_relative(relative) {
        return Err(failure());
    }
    let mut p = root.to_path_buf();
    let segments: Vec<_> = relative.split('/').collect();
    for (n, s) in segments.iter().enumerate() {
        p.push(s);
        match fs::symlink_metadata(&p) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(failure()),
            Ok(_) => {}
        }
        let i = path_identity(&p)?;
        if i.is_symlink || (n + 1 < segments.len() && !i.is_dir) {
            return Err(failure());
        }
    }
    read_selected(&p).map(Some)
}
#[derive(Clone)]
struct Reference {
    path: String,
    blocking: bool,
}
fn references(body: &str) -> Vec<Reference> {
    let mut found = Vec::new();
    let dir =
        regex::Regex::new(r"(?i)^(?:\.\.?[/\\]|(?:references?|assets?)[/\\])|\$\{skill_dir\}")
            .unwrap();
    let external = regex::Regex::new(r"(?i)^(?:[a-z][a-z0-9+.-]*:|#|/)").unwrap();
    for pattern in [
        r"\]\(\s*(<?)([^)\s>]+)",
        r"(?m)^ {0,3}\[[^^\]\n][^\]\n]*\]:[ \t]*(<?)([^\s>]+)",
    ] {
        for c in regex::Regex::new(pattern).unwrap().captures_iter(body) {
            let p = &c[2];
            if external.is_match(p) {
                continue;
            }
            let blocking = if &c[1] == "<" {
                p.to_ascii_lowercase().contains("${skill_dir}")
            } else {
                dir.is_match(p)
            };
            found.push((
                c.get(0).unwrap().start(),
                Reference {
                    path: p.into(),
                    blocking,
                },
            ));
        }
    }
    for c in regex::Regex::new(r#"(?im)(?:^|[\s\x60'"(])((?:\$\{skill_dir\}[/\\])?(?:references?|assets?)[/\\][^\s\x60'")]+)"#).unwrap().captures_iter(body){found.push((c.get(0).unwrap().start(),Reference{path:c[1].into(),blocking:true}));}
    for c in regex::Regex::new(r#"(?i)\$\{skill_dir\}(?:[/\\][^\s\x60'")]+)?"#)
        .unwrap()
        .captures_iter(body)
    {
        found.push((
            c.get(0).unwrap().start(),
            Reference {
                path: c[0].into(),
                blocking: true,
            },
        ));
    }
    found.sort_by_key(|x| x.0);
    found.into_iter().map(|x| x.1).collect()
}
fn resolve(reference: &str, base: &[String]) -> Option<String> {
    let root = reference.to_ascii_lowercase().starts_with("${skill_dir}");
    let input = if root {
        reference[12..].trim_start_matches(['/', '\\'])
    } else {
        reference
    };
    let cleaned: String = input
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || "._/\\-".contains(*c))
        .collect();
    let cleaned = cleaned.trim_end_matches('.').replace('\\', "/");
    if cleaned.is_empty() {
        return None;
    }
    let mut parts = if root { vec![] } else { base.to_vec() };
    for p in cleaned.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(p.into()),
        }
    }
    let path = parts.join("/");
    (safe_relative(&path) && text_extension(&path)).then_some(path)
}
fn bundle(
    body: &str,
    mut read: impl FnMut(&str) -> Option<String>,
) -> (Vec<BundledFile>, bool, Vec<String>) {
    let mut files = Vec::new();
    let mut included = HashSet::from(["skill.md".to_string()]);
    let mut missing = Vec::new();
    let mut blocked = false;
    let mut queue: VecDeque<_> = references(body)
        .into_iter()
        .map(|r| (r, Vec::<String>::new()))
        .collect();
    while let Some((r, base)) = queue.pop_front() {
        let explicit = ["./", "../", ".\\", "..\\"]
            .iter()
            .any(|p| r.path.starts_with(p));
        let mut paths: Vec<_> = resolve(&r.path, &base).into_iter().collect();
        if !explicit {
            if let Some(p) = resolve(&r.path, &[]) {
                if !paths.contains(&p) {
                    paths.push(p)
                }
            }
        }
        let mut found = None;
        let mut already = false;
        for p in paths {
            if included.contains(&p.to_ascii_lowercase()) {
                already = true;
                break;
            }
            if let Some(content) = read(&p) {
                found = Some(BundledFile { path: p, content });
                break;
            }
        }
        if already {
            continue;
        }
        let Some(file) = found.filter(|_| files.len() < 16) else {
            blocked |= r.blocking;
            let shown = r
                .path
                .split(['#', '?'])
                .next()
                .unwrap_or_default()
                .to_string();
            if !r.blocking && !shown.is_empty() && !missing.contains(&shown) {
                missing.push(shown)
            }
            continue;
        };
        included.insert(file.path.to_ascii_lowercase());
        let base = file
            .path
            .split('/')
            .take(file.path.split('/').count() - 1)
            .map(str::to_string)
            .collect::<Vec<_>>();
        queue.extend(
            references(&file.content)
                .into_iter()
                .map(|r| (r, base.clone())),
        );
        files.push(BundledFile {
            path: file.path,
            content: file.content.trim().into(),
        });
    }
    (files, blocked, missing)
}
fn inspect_package(
    raw: &str,
    fallback: &str,
    read: impl FnMut(&str) -> Option<String>,
) -> Result<PackageInspection, String> {
    let (mut fields, body) = parse_frontmatter_mode(raw, true);
    if fields.get("name").is_none_or(|v| v.trim().is_empty()) {
        fields.insert("name".into(), fallback.into());
    }
    let (files, blocked, missing) = bundle(&body, read);
    let content = std::iter::once(body)
        .chain(
            files
                .iter()
                .map(|f| format!("---\n\n## {}\n\n{}", f.path, f.content)),
        )
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(PackageInspection {
        skill: inspect_skill(raw, fields, content, blocked)?,
        bundled_files: files.into_iter().map(|f| f.path).collect(),
        unbundled_references: missing,
    })
}
pub fn inspect_installed(
    raw: &str,
    bundle_state: &InstalledBundle,
    fallback: &str,
) -> Result<WritingSkillInspection, String> {
    if matches!(bundle_state, InstalledBundle::None) {
        return inspect_writing_skill_markdown(raw);
    }
    let files = match bundle_state {
        InstalledBundle::Intact { files } => files.as_slice(),
        _ => &[],
    };
    let (mut fields, body) = parse_frontmatter_mode(raw, true);
    if fields.get("name").is_none_or(|v| v.trim().is_empty()) {
        fields.insert("name".into(), fallback.into());
    }
    let (_, blocked, _) = bundle(&body, |p| {
        files
            .iter()
            .find(|f| f.path.eq_ignore_ascii_case(p))
            .map(|f| f.content.clone())
    });
    let content = std::iter::once(body)
        .chain(
            files
                .iter()
                .map(|f| format!("---\n\n## {}\n\n{}", f.path, f.content.trim())),
        )
        .collect::<Vec<_>>()
        .join("\n\n");
    let mut result = inspect_skill(raw, fields, content, blocked)?;
    if matches!(bundle_state, InstalledBundle::Damaged) {
        result.compatible = false;
        result.reasons.push("bundle-damaged".into())
    }
    Ok(result)
}
pub fn read_installed_bundle(
    mut read: impl FnMut(&str) -> Result<Option<Vec<u8>>, String>,
) -> InstalledBundle {
    let attempt = (|| -> Result<InstalledBundle, String> {
        let Some(bytes) = read(BUNDLE_MANIFEST)? else {
            return Ok(InstalledBundle::None);
        };
        let m: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| failure())?;
        if m["version"].as_u64() != Some(1) {
            return Err(failure());
        }
        let entries = m["files"].as_array().ok_or_else(failure)?;
        if entries.len() > 16 {
            return Err(failure());
        }
        let mut seen = HashSet::from(["skill.md".to_string()]);
        let mut files = Vec::new();
        let mut total = 0;
        for e in entries {
            let p = e["path"].as_str().ok_or_else(failure)?;
            let hash = e["sha256"].as_str().ok_or_else(failure)?;
            if !safe_relative(p)
                || !text_extension(p)
                || !seen.insert(p.to_ascii_lowercase())
                || hash.len() != 64
                || !hash
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err(failure());
            }
            let bytes = read(p)?.ok_or_else(failure)?;
            total += bytes.len();
            if total > MAX_SKILL_BYTES || bytes.contains(&0) || digest(&bytes) != hash {
                return Err(failure());
            }
            files.push(BundledFile {
                path: p.into(),
                content: String::from_utf8(bytes).map_err(|_| failure())?,
            });
        }
        Ok(InstalledBundle::Intact { files })
    })();
    attempt.unwrap_or(InstalledBundle::Damaged)
}
fn folder_files(
    root: &Path,
    prefix: &str,
    depth: usize,
    visited: &mut usize,
    files: &mut Vec<String>,
) -> Result<(), String> {
    let dir = root.join(prefix);
    let before = path_identity(&dir)?;
    if before.is_symlink || !before.is_dir {
        return Err(failure());
    }
    let mut entries = fs::read_dir(&dir)
        .map_err(|_| failure())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| failure())?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().into_string().map_err(|_| failure())?;
        if name.starts_with('.') {
            continue;
        }
        *visited += 1;
        if *visited > 200 {
            return Err(failure());
        }
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let i = path_identity(&e.path())?;
        if i.is_symlink {
            return Err(failure());
        }
        if i.is_dir {
            if depth >= 4 {
                return Err(failure());
            }
            folder_files(root, &rel, depth + 1, visited, files)?
        } else if i.is_file {
            files.push(rel)
        } else {
            return Err(failure());
        }
    }
    if path_identity(&dir)? != before {
        return Err(failure());
    }
    Ok(())
}
/// 仅原生对话框回调传入路径，渲染层只持有候选 id。
pub fn load_local(kind: &str, path: &Path) -> Result<LoadedPackage, String> {
    if !path.is_absolute() || !matches!(kind, "file" | "folder") {
        return Err(failure());
    }
    let info = path_identity(path)?;
    if info.is_symlink {
        return Err(failure());
    }
    let (root, filename, fallback, all) = if kind == "file" {
        if !info.is_file
            || !["md", "markdown"].contains(
                &path
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
            )
        {
            return Err(failure());
        }
        let root = path.parent().ok_or_else(failure)?.to_path_buf();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(failure)?
            .to_string();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        let fallback = if stem.eq_ignore_ascii_case("skill") {
            root.file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
        } else {
            stem
        }
        .to_string();
        (root, name, fallback, Vec::new())
    } else {
        if !info.is_dir {
            return Err(failure());
        }
        let mut all = Vec::new();
        folder_files(path, "", 1, &mut 0, &mut all)?;
        let name = all
            .iter()
            .find(|p| p.eq_ignore_ascii_case("skill.md"))
            .cloned()
            .ok_or_else(failure)?;
        (
            path.to_path_buf(),
            name,
            path.file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(failure)?
                .to_string(),
            all,
        )
    };
    let raw = String::from_utf8(read_selected(&root.join(&filename))?)
        .map_err(|_| failure())?
        .trim_start_matches('\u{feff}')
        .to_string();
    let mut read_files = HashMap::new();
    let inspection = inspect_package(&raw, &fallback, |p| {
        if kind != "folder" {
            return None;
        }
        let bytes = read_file(&root, p).ok().flatten()?;
        let text = String::from_utf8(bytes)
            .ok()?
            .trim_start_matches('\u{feff}')
            .to_string();
        read_files.insert(p.to_string(), text.clone());
        Some(text)
    })?;
    if !regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
        .unwrap()
        .is_match(&inspection.skill.metadata.name)
    {
        return Err(failure());
    }
    let files: Vec<_> = inspection
        .bundled_files
        .iter()
        .map(|p| BundledFile {
            path: p.clone(),
            content: read_files[p].clone(),
        })
        .collect();
    let content_sha256 =
        digest(&serde_json::to_vec(&(raw.clone(), &files)).map_err(|_| failure())?);
    let skipped_files = all
        .into_iter()
        .filter(|p| p != &filename && !files.iter().any(|f| f.path.eq_ignore_ascii_case(p)))
        .collect();
    Ok(LoadedPackage {
        raw,
        inspection,
        files,
        skipped_files,
        source_name: path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(failure)?
            .into(),
        content_sha256,
    })
}
/// 清单先写；失败保留损坏状态，不递归删除未经复核的目录。
pub fn install_at(dir: &Path, loaded: &LoadedPackage) -> Result<(), String> {
    if !loaded.inspection.skill.compatible {
        return Err("Skill 不兼容 / Incompatible skill".into());
    }
    safe_root(dir.parent().ok_or_else(failure)?)?;
    fs::create_dir(dir).map_err(|_| {
        "Skill 已存在或无法写入 / Skill already exists or cannot be written".to_string()
    })?;
    let identity = path_identity(dir)?;
    let verify = || -> Result<(), String> {
        let i = path_identity(dir)?;
        if i.is_symlink || !i.is_dir || i.dev != identity.dev || i.ino != identity.ino {
            Err(failure())
        } else {
            Ok(())
        }
    };
    let write = |relative: &str, bytes: &[u8]| -> Result<(), String> {
        verify()?;
        safe_root(dir)?;
        if !safe_relative(relative) {
            return Err(failure());
        }
        let mut cursor = dir.to_path_buf();
        let parts: Vec<_> = relative.split('/').collect();
        for part in &parts[..parts.len() - 1] {
            cursor.push(part);
            match fs::create_dir(&cursor) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(failure()),
            }
            let i = path_identity(&cursor)?;
            if i.is_symlink || !i.is_dir {
                return Err(failure());
            }
        }
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(dir.join(relative)).map_err(|_| failure())?;
        file.write_all(bytes).map_err(|_| failure())?;
        file.sync_all().map_err(|_| failure())?;
        verify()
    };
    let manifest = serde_json::json!({"version":1,"files":loaded.files.iter().map(|f|serde_json::json!({"path":f.path,"sha256":digest(f.content.as_bytes())})).collect::<Vec<_>>()});
    write(
        BUNDLE_MANIFEST,
        &serde_json::to_vec(&manifest).map_err(|_| failure())?,
    )?;
    write("SKILL.md", loaded.raw.as_bytes())?;
    for f in &loaded.files {
        write(&f.path, f.content.as_bytes())?
    }
    if matches!(
        read_installed_bundle(|p| read_file(dir, p)),
        InstalledBundle::Damaged
    ) {
        return Err(failure());
    }
    Ok(())
}
#[derive(Debug)]
pub struct Candidate {
    id: String,
    kind: String,
    path: PathBuf,
    hash: String,
    created: Instant,
}
#[derive(Debug, Default)]
pub struct Candidates {
    requests: HashMap<String, String>,
    candidates: HashMap<String, Candidate>,
}
impl Candidates {
    pub fn begin(&mut self, owner: &str) -> String {
        self.candidates.remove(owner);
        let id = crate::project_access::random_uuid_v4();
        self.requests.insert(owner.into(), id.clone());
        id
    }
    pub fn complete(
        &mut self,
        owner: &str,
        request: &str,
        kind: &str,
        path: PathBuf,
        loaded: &LoadedPackage,
    ) -> Result<serde_json::Value, String> {
        if self.requests.get(owner).map(String::as_str) != Some(request) {
            return Err(failure());
        }
        self.requests.remove(owner);
        let id = crate::project_access::random_uuid_v4();
        self.candidates.insert(
            owner.into(),
            Candidate {
                id: id.clone(),
                kind: kind.into(),
                path,
                hash: loaded.content_sha256.clone(),
                created: Instant::now(),
            },
        );
        let mut value = serde_json::to_value(&loaded.inspection).map_err(|_| failure())?;
        let obj = value.as_object_mut().ok_or_else(failure)?;
        obj.remove("content");
        obj.insert("candidateId".into(), id.into());
        obj.insert("sourceKind".into(), kind.into());
        obj.insert("sourceName".into(), loaded.source_name.clone().into());
        obj.insert("contentSha256".into(), loaded.content_sha256.clone().into());
        obj.insert(
            "skippedFiles".into(),
            serde_json::to_value(&loaded.skipped_files).map_err(|_| failure())?,
        );
        Ok(value)
    }
    pub fn take(&mut self, owner: &str, id: &str) -> Result<LoadedPackage, String> {
        let entry = self.candidates.get(owner).ok_or_else(failure)?;
        if entry.id != id {
            return Err(failure());
        }
        let entry = self.candidates.remove(owner).ok_or_else(failure)?;
        if entry.created.elapsed() > Duration::from_secs(600) {
            return Err(failure());
        }
        let loaded = load_local(&entry.kind, &entry.path)?;
        if loaded.content_sha256 != entry.hash {
            return Err("Skill 在检查后变化，请重新选择 / Skill changed after inspection".into());
        }
        Ok(loaded)
    }
    pub fn clear(&mut self, owner: &str) {
        self.requests.remove(owner);
        self.candidates.remove(owner);
    }
}
#[cfg(test)]
mod tests;
