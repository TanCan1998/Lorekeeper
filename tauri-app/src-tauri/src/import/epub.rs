//! EPUB 纯内存提取：平移 Electron 的阅读顺序、字体混淆例外与解压限额。
//! 只读取调用方已授权取得的字节；不落地条目、不执行 HTML、不访问网络。

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::OnceLock;

use regex::{Captures, Regex};
use zip::ZipArchive;

pub(crate) const EPUB_MAX_ARCHIVE_ENTRIES: usize = 10_000;
pub(crate) const EPUB_MAX_ENTRY_BYTES: usize = 32 * 1024 * 1024;
pub(crate) const EPUB_MAX_EXTRACTED_BYTES: usize = 128 * 1024 * 1024;

const INVALID: &str = "EPUB_INVALID_ARCHIVE";
const EXPANSION_LIMIT: &str = "EPUB_EXPANSION_LIMIT";
const DRM_UNSUPPORTED: &str = "EPUB_DRM_UNSUPPORTED";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EpubChapter {
    pub title: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct EpubLimits {
    pub max_entries: usize,
    pub max_entry_bytes: usize,
    pub max_extracted_bytes: usize,
}

impl Default for EpubLimits {
    fn default() -> Self {
        Self {
            max_entries: EPUB_MAX_ARCHIVE_ENTRIES,
            max_entry_bytes: EPUB_MAX_ENTRY_BYTES,
            max_extracted_bytes: EPUB_MAX_EXTRACTED_BYTES,
        }
    }
}

// 每个调用点独立缓存固定正则；不从归档内容构造表达式，不使用回溯引擎。
macro_rules! epub_regex {
    ($pattern:literal) => {{
        static PATTERN: OnceLock<Regex> = OnceLock::new();
        PATTERN.get_or_init(|| Regex::new($pattern).expect("固定 EPUB 正则"))
    }};
}

/// ZIP 内部 POSIX 路径，与基线相同地归约点片段；绝不映射到本机文件系统。
fn normalize_archive_path(value: &str) -> Result<String, String> {
    if value.is_empty() || value.contains(['\0', '\\']) {
        return Err(INVALID.to_string());
    }
    let mut segments = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(INVALID.to_string());
                }
            }
            _ => segments.push(part),
        }
    }
    if segments.is_empty() {
        return Err(INVALID.to_string());
    }
    let mut normalized = segments.join("/");
    if value.ends_with('/') {
        normalized.push('/');
    }
    Ok(normalized)
}

fn decode_reference(value: &str) -> Result<String, String> {
    let relative = value.split(['?', '#']).next().unwrap_or_default();
    if relative.is_empty() {
        return Err(INVALID.to_string());
    }
    let source = relative.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'%' {
            let high = source
                .get(index + 1)
                .and_then(|byte| (*byte as char).to_digit(16));
            let low = source
                .get(index + 2)
                .and_then(|byte| (*byte as char).to_digit(16));
            match (high, low) {
                (Some(high), Some(low)) => decoded.push((high * 16 + low) as u8),
                _ => return Err(INVALID.to_string()),
            }
            index += 3;
        } else {
            decoded.push(source[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| INVALID.to_string())
}

fn resolve_manifest_path(directory: &str, href: &str) -> Result<String, String> {
    normalize_archive_path(&format!("{directory}/{}", decode_reference(href)?))
}

fn decode_entities(value: &str) -> String {
    epub_regex!(r"(?i)&(#x[0-9a-f]+|#[0-9]+|[a-z]+);")
        .replace_all(value, |captures: &Captures<'_>| {
            let body = captures[1].to_ascii_lowercase();
            let named = match body.as_str() {
                "amp" => Some("&"),
                "apos" => Some("'"),
                "gt" => Some(">"),
                "lt" => Some("<"),
                "nbsp" => Some(" "),
                "quot" => Some("\""),
                _ => None,
            };
            if let Some(decoded) = named {
                return decoded.to_string();
            }
            let numeric = if let Some(hex) = body.strip_prefix("#x") {
                u32::from_str_radix(hex, 16).ok()
            } else {
                body.strip_prefix('#')
                    .and_then(|decimal| decimal.parse::<u32>().ok())
            };
            numeric
                .and_then(char::from_u32)
                .map(|ch| ch.to_string())
                .unwrap_or_else(|| captures[0].to_string())
        })
        .into_owned()
}

fn attributes(source: &str) -> HashMap<String, String> {
    epub_regex!(r#"([\w:-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#)
        .captures_iter(source)
        .map(|captures| {
            let value = captures
                .get(2)
                .or_else(|| captures.get(3))
                .expect("属性正则必有一个引号分支")
                .as_str();
            (captures[1].to_ascii_lowercase(), decode_entities(value))
        })
        .collect()
}

fn all_tag_attributes(xml: &str, pattern: &Regex) -> Vec<HashMap<String, String>> {
    pattern
        .captures_iter(xml)
        .map(|captures| attributes(&captures[1]))
        .collect()
}

fn first_tag_attributes(xml: &str, pattern: &Regex) -> Option<HashMap<String, String>> {
    pattern
        .captures(xml)
        .map(|captures| attributes(&captures[1]))
}

fn first_content<'a>(document: &'a str, pattern: &Regex) -> Option<&'a str> {
    pattern
        .captures(document)
        .and_then(|captures| captures.get(1))
        .map(|value| value.as_str())
}

fn html_to_text(html: &str) -> String {
    let text = epub_regex!(r"(?s)<!--[\s\S]*?-->").replace_all(html, "");
    let text = epub_regex!(r"(?is)<script\b[^>]*>.*?</script>").replace_all(&text, "");
    let text = epub_regex!(r"(?is)<style\b[^>]*>.*?</style>").replace_all(&text, "");
    let text = epub_regex!(r"(?i)<br\s*/?>").replace_all(&text, "\n");
    let text = epub_regex!(r"(?i)</?(?:address|article|aside|blockquote|div|h[1-6]|header|li|main|nav|ol|p|pre|section|table|tr|ul)\b[^>]*>")
        .replace_all(&text, "\n\n");
    let text = epub_regex!(r"<[^>]+>").replace_all(&text, "");
    let text = decode_entities(&text)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let text = text
        .split('\n')
        .map(|line| {
            epub_regex!(r"[\t ]+")
                .replace_all(line, " ")
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    epub_regex!(r"\n{3,}")
        .replace_all(&text, "\n\n")
        .trim()
        .to_string()
}

fn extract_document(document: &str, document_path: &str) -> EpubChapter {
    let body = first_content(
        document,
        epub_regex!(r"(?is)<(?:[\w-]+:)?body\b[^>]*>(.*?)</(?:[\w-]+:)?body>"),
    )
    .unwrap_or(document);
    let heading = first_content(document, epub_regex!(r"(?is)<h[1-6]\b[^>]*>(.*?)</h[1-6]>"));
    let title = first_content(
        document,
        epub_regex!(r"(?is)<(?:[\w-]+:)?title\b[^>]*>(.*?)</(?:[\w-]+:)?title>"),
    );
    let title = html_to_text(heading.or(title).unwrap_or_default());
    let file_name = document_path.rsplit('/').next().unwrap_or(document_path);
    let fallback = match file_name.rfind('.') {
        Some(index) if index > 0 => &file_name[..index],
        _ => file_name,
    };
    EpubChapter {
        title: if title.is_empty() {
            fallback.to_string()
        } else {
            title
        },
        content: html_to_text(body),
    }
}

/// 严格解码，与基线 TextDecoder fatal 模式一致；UTF-16 BOM 或 XML 起始字节决定端序。
fn decode_document(bytes: &[u8]) -> Result<String, String> {
    let (utf16, little_endian, offset) = if bytes.starts_with(&[0xff, 0xfe]) {
        (true, true, 2)
    } else if bytes.starts_with(&[0xfe, 0xff]) {
        (true, false, 2)
    } else if bytes.starts_with(&[b'<', 0, b'?', 0]) {
        (true, true, 0)
    } else if bytes.starts_with(&[0, b'<', 0, b'?']) {
        (true, false, 0)
    } else {
        (false, false, 0)
    };
    if utf16 {
        let bytes = &bytes[offset..];
        if bytes.len() % 2 != 0 {
            return Err(INVALID.to_string());
        }
        let units = bytes
            .chunks_exact(2)
            .map(|pair| {
                if little_endian {
                    u16::from_le_bytes([pair[0], pair[1]])
                } else {
                    u16::from_be_bytes([pair[0], pair[1]])
                }
            })
            .collect::<Vec<_>>();
        String::from_utf16(&units).map_err(|_| INVALID.to_string())
    } else {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        std::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| INVALID.to_string())
    }
}

fn read_document(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    index: usize,
    limits: EpubLimits,
    consumed: &mut usize,
) -> Result<String, String> {
    let mut entry = archive.by_index(index).map_err(|_| INVALID.to_string())?;
    let limit = limits
        .max_entry_bytes
        .min(limits.max_extracted_bytes.saturating_sub(*consumed));
    if entry.size() > limit as u64 {
        return Err(EXPANSION_LIMIT.to_string());
    }
    let declared_size = entry.size();
    let mut bytes = Vec::new();
    // 不能仅信任中央目录声明；额外读一字节才能区分刚好到达上限与真正超限。
    (&mut entry)
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| INVALID.to_string())?;
    if bytes.len() > limit {
        return Err(EXPANSION_LIMIT.to_string());
    }
    if bytes.len() as u64 != declared_size {
        return Err(INVALID.to_string());
    }
    *consumed += bytes.len();
    decode_document(&bytes)
}

struct ManifestItem {
    archive_path: String,
    media_type: String,
    properties: String,
}

fn is_font_media_type(media_type: &str) -> bool {
    let normalized = media_type.trim().to_ascii_lowercase();
    normalized.starts_with("font/")
        || matches!(
            normalized.as_str(),
            "application/font-sfnt"
                | "application/font-woff"
                | "application/vnd.ms-fontobject"
                | "application/vnd.ms-opentype"
                | "application/x-font-opentype"
                | "application/x-font-ttf"
        )
}

fn assert_supported_encryption(
    xml: &str,
    manifest: &HashMap<String, ManifestItem>,
    paths: &HashMap<String, String>,
    entries: &HashMap<String, usize>,
) -> Result<(), String> {
    let declarations = epub_regex!(r"(?i)<(?:[\w-]+:)?EncryptedData\b")
        .find_iter(xml)
        .count();
    let blocks =
        epub_regex!(r"(?is)<(?:[\w-]+:)?EncryptedData\b[^>]*>(.*?)</(?:[\w-]+:)?EncryptedData\s*>")
            .captures_iter(xml)
            .collect::<Vec<_>>();
    if declarations == 0 || blocks.len() != declarations {
        return Err(INVALID.to_string());
    }
    for block in blocks {
        let method = first_tag_attributes(
            &block[1],
            epub_regex!(r"(?i)<(?:[\w-]+:)?EncryptionMethod\b([^>]*)>"),
        );
        let reference = first_tag_attributes(
            &block[1],
            epub_regex!(r"(?i)<(?:[\w-]+:)?CipherReference\b([^>]*)>"),
        );
        let algorithm = method
            .as_ref()
            .and_then(|attrs| attrs.get("algorithm"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        let uri = reference
            .as_ref()
            .and_then(|attrs| attrs.get("uri"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty());
        let (Some(algorithm), Some(uri)) = (algorithm, uri) else {
            return Err(INVALID.to_string());
        };
        let path = normalize_archive_path(&decode_reference(uri)?)?;
        let item = paths.get(&path).and_then(|id| manifest.get(id));
        if !matches!(
            algorithm,
            "http://www.idpf.org/2008/embedding" | "http://ns.adobe.com/pdf/enc#RC"
        ) || !item.is_some_and(|item| is_font_media_type(&item.media_type))
            || !entries.contains_key(&path)
        {
            return Err(DRM_UNSUPPORTED.to_string());
        }
    }
    Ok(())
}

/// zip 9.0.3 会按原始文件名合并中央目录条目，不能单靠 len 判断重复与条目限额。
/// 这里只遍历已由 zip 定位的中央目录定长头和长度字段；ZIP64、解压及 CRC 仍交给 zip。
fn assert_central_directory_count(
    bytes: &[u8],
    directory_start: u64,
    indexed_count: usize,
    max_entries: usize,
) -> Result<(), String> {
    let mut offset = usize::try_from(directory_start).map_err(|_| INVALID.to_string())?;
    let mut count = 0usize;
    while bytes
        .get(offset..)
        .is_some_and(|tail| tail.starts_with(b"PK\x01\x02"))
    {
        let header_end = offset.checked_add(46).ok_or_else(|| INVALID.to_string())?;
        let header = bytes
            .get(offset..header_end)
            .ok_or_else(|| INVALID.to_string())?;
        count += 1;
        if count > max_entries {
            return Err(EXPANSION_LIMIT.to_string());
        }
        if count > indexed_count {
            return Err(INVALID.to_string());
        }
        let variable_bytes: usize = [28, 30, 32]
            .into_iter()
            .map(|index| u16::from_le_bytes([header[index], header[index + 1]]) as usize)
            .sum();
        offset = header_end
            .checked_add(variable_bytes)
            .ok_or_else(|| INVALID.to_string())?;
        if offset > bytes.len() {
            return Err(INVALID.to_string());
        }
    }
    if count != indexed_count {
        return Err(INVALID.to_string());
    }
    Ok(())
}

/// 只提取 OPF spine 的正文；图片、样式与排版资源不会解包或成为正文。
pub(crate) fn extract_epub_chapters(
    bytes: &[u8],
    limits: EpubLimits,
) -> Result<Vec<EpubChapter>, String> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| INVALID.to_string())?;
    assert_central_directory_count(
        bytes,
        archive.central_directory_start(),
        archive.len(),
        limits.max_entries,
    )?;
    let mut entries = HashMap::new();
    for index in 0..archive.len() {
        // 原始条目读取不解压、不尝试解密，先对所有条目统一检查加密标志。
        let entry = archive
            .by_index_raw(index)
            .map_err(|_| INVALID.to_string())?;
        let name = entry.name().map_err(|_| INVALID.to_string())?;
        let name = normalize_archive_path(&name)?;
        if entry.encrypted() {
            return Err(DRM_UNSUPPORTED.to_string());
        }
        if entries.insert(name, index).is_some() {
            return Err(INVALID.to_string());
        }
    }
    let mut consumed = 0;
    let container_index = *entries
        .get("META-INF/container.xml")
        .ok_or_else(|| INVALID.to_string())?;
    let container = read_document(&mut archive, container_index, limits, &mut consumed)?;
    let rootfile = first_tag_attributes(
        &container,
        epub_regex!(r"(?i)<(?:[\w-]+:)?rootfile\b([^>]*)>"),
    )
    .ok_or_else(|| INVALID.to_string())?;
    let package_path = normalize_archive_path(
        rootfile
            .get("full-path")
            .ok_or_else(|| INVALID.to_string())?,
    )?;
    let package_index = *entries
        .get(&package_path)
        .ok_or_else(|| INVALID.to_string())?;
    let opf = read_document(&mut archive, package_index, limits, &mut consumed)?;
    let directory = package_path
        .rsplit_once('/')
        .map(|(directory, _)| directory)
        .unwrap_or_default();
    let mut manifest = HashMap::new();
    let mut paths = HashMap::new();
    for item in all_tag_attributes(&opf, epub_regex!(r"(?i)<(?:[\w-]+:)?item\b([^>]*)>")) {
        let id = item
            .get("id")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| INVALID.to_string())?;
        let href = item
            .get("href")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| INVALID.to_string())?;
        let archive_path = resolve_manifest_path(directory, href)?;
        if manifest.contains_key(id) || paths.insert(archive_path.clone(), id.clone()).is_some() {
            return Err(INVALID.to_string());
        }
        manifest.insert(
            id.clone(),
            ManifestItem {
                archive_path,
                media_type: item.get("media-type").cloned().unwrap_or_default(),
                properties: item.get("properties").cloned().unwrap_or_default(),
            },
        );
    }
    if let Some(index) = entries.get("META-INF/encryption.xml") {
        let encryption = read_document(&mut archive, *index, limits, &mut consumed)?;
        assert_supported_encryption(&encryption, &manifest, &paths, &entries)?;
    }
    let mut chapters = Vec::new();
    for itemref in all_tag_attributes(&opf, epub_regex!(r"(?i)<(?:[\w-]+:)?itemref\b([^>]*)>")) {
        if itemref
            .get("linear")
            .is_some_and(|linear| linear.eq_ignore_ascii_case("no"))
        {
            continue;
        }
        let item = itemref
            .get("idref")
            .and_then(|id| manifest.get(id))
            .ok_or_else(|| INVALID.to_string())?;
        if !matches!(
            item.media_type.to_ascii_lowercase().as_str(),
            "application/xhtml+xml" | "text/html"
        ) || item
            .properties
            .split_whitespace()
            .any(|property| property == "nav")
        {
            continue;
        }
        let index = *entries
            .get(&item.archive_path)
            .ok_or_else(|| INVALID.to_string())?;
        let document = read_document(&mut archive, index, limits, &mut consumed)?;
        let chapter = extract_document(&document, &item.archive_path);
        if !chapter.content.is_empty() {
            chapters.push(chapter);
        }
    }
    if chapters.is_empty() {
        return Err(INVALID.to_string());
    }
    Ok(chapters)
}

#[cfg(test)]
mod tests;
