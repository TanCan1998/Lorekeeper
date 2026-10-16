//! WebDAV 追加式归档：archive → manifest → completion，完成标记最后发布。
//! 不覆盖远端文件，所有PUT带If-None-Match:*且读回校验；拒绝重定向、越域href与不完整代。
//! 仅复用reqwest native-tls+socks、tokio与既有ZIP归档，不增加网络/密码学依赖。
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use reqwest::{Client, Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use url::Url;

use super::cloud_credential_store::{normalize_webdav_endpoint, valid_account_id};
use super::cloud_project_binding_store::safe_id;
use super::portable_project_archive as files;

pub type CancelFlag = Arc<AtomicBool>;
pub fn cancellation() -> CancelFlag {
    Arc::new(AtomicBool::new(false))
}
fn error(code: &str) -> String {
    code.to_string()
}
pub fn check_cancel(cancel: &CancelFlag) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        Err(error("WEBDAV_CANCELLED"))
    } else {
        Ok(())
    }
}
async fn cancel_wait(cancel: &CancelFlag) {
    while !cancel.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
async fn checked<T>(
    cancel: &CancelFlag,
    future: impl std::future::Future<Output = Result<T, reqwest::Error>>,
) -> Result<T, String> {
    check_cancel(cancel)?;
    tokio::select! {
        _=cancel_wait(cancel)=>Err(error("WEBDAV_CANCELLED")),
        result=future=>result.map_err(|_|error("WEBDAV_NETWORK")),
    }
}
/// 不实现Debug/Serialize，避免Basic认证信息进入日志或IPC响应。
pub struct WebDavAccount {
    pub endpoint: String,
    pub username: String,
    pub secret: String,
}
impl WebDavAccount {
    fn endpoint(&self) -> Result<Url, String> {
        if self.username.is_empty()
            || self.secret.is_empty()
            || self
                .username
                .chars()
                .chain(self.secret.chars())
                .any(char::is_control)
        {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        Url::parse(
            &normalize_webdav_endpoint(&self.endpoint)
                .map_err(|_| error("WEBDAV_INPUT_INVALID"))?,
        )
        .map_err(|_| error("WEBDAV_INPUT_INVALID"))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebDavGeneration {
    pub cloud_book_id: String,
    pub generation_id: String,
    pub parent_generation_ids: Vec<String>,
    pub created_at: String,
    pub origin_project_id: String,
    pub portable_snapshot_generation: String,
    pub archive_sha256: String,
    pub archive_byte_size: u64,
    pub has_sibling: bool,
    pub sibling_generation_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Descriptor {
    file: String,
    sha256: String,
    byte_size: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    version: u32,
    cloud_book_id: String,
    generation_id: String,
    parent_generation_ids: Vec<String>,
    created_at: String,
    origin_project_id: String,
    portable_snapshot_generation: String,
    archive: Descriptor,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Completion {
    version: u32,
    cloud_book_id: String,
    generation_id: String,
    manifest: Descriptor,
    archive: Descriptor,
    completed_at: String,
}
#[derive(Clone)]
pub struct WebDavService {
    client: Client,
    pub max_archive_bytes: u64,
}
impl WebDavService {
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| error("WEBDAV_NETWORK"))?;
        Ok(Self {
            client,
            max_archive_bytes: 16 * 1024 * 1024 * 1024,
        })
    }
    fn location(
        endpoint: &Url,
        book: &str,
        generation: Option<&str>,
        file: Option<&str>,
    ) -> Result<Url, String> {
        if !safe_id(book) || generation.is_some_and(|id| !valid_account_id(id)) {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        let mut result = endpoint.clone();
        {
            let mut path = result
                .path_segments_mut()
                .map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
            path.pop_if_empty()
                .push("books")
                .push(book)
                .push("generations");
            if let Some(generation) = generation {
                path.push(generation);
            }
            if let Some(file) = file {
                path.push(file);
            } else {
                path.push("");
            }
        }
        Ok(result)
    }
    async fn request(
        &self,
        account: &WebDavAccount,
        url: Url,
        method: Method,
        body: Option<reqwest::Body>,
        length: Option<u64>,
        depth: Option<&str>,
        cancel: &CancelFlag,
        transfer: bool,
    ) -> Result<Response, String> {
        let endpoint = account.endpoint()?;
        if url.origin() != endpoint.origin()
            || !url.path().starts_with(endpoint.path())
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        let mut request = self
            .client
            .request(method.clone(), url)
            .basic_auth(&account.username, Some(&account.secret))
            .timeout(Duration::from_secs(if transfer { 30 * 60 } else { 15 }));
        if let Some(depth) = depth {
            request = request
                .header("Depth", depth)
                .header("Accept", "application/xml");
        }
        if method == Method::PUT || method.as_str() == "MKCOL" {
            request = request.header("If-None-Match", "*");
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        if let Some(length) = length {
            request = request.header("Content-Length", length);
        }
        checked(cancel, request.send()).await
    }
    pub async fn check_connection(
        &self,
        account: &WebDavAccount,
        cancel: &CancelFlag,
    ) -> Result<(), String> {
        let response = self
            .request(
                account,
                account.endpoint()?,
                method("PROPFIND")?,
                None,
                None,
                Some("0"),
                cancel,
                false,
            )
            .await?;
        good(&response)?;
        read_bytes(response, 1024 * 1024, cancel).await?;
        Ok(())
    }
    async fn existing(
        &self,
        account: &WebDavAccount,
        url: &Url,
        expected: &Descriptor,
        allow_missing: bool,
        cancel: &CancelFlag,
    ) -> Result<bool, String> {
        let response = self
            .request(
                account,
                url.clone(),
                Method::GET,
                None,
                None,
                None,
                cancel,
                true,
            )
            .await?;
        if response.status() == StatusCode::NOT_FOUND && allow_missing {
            return Ok(false);
        }
        good(&response)?;
        let actual = hash_response(
            response,
            expected.byte_size.max(1).min(self.max_archive_bytes),
            cancel,
        )
        .await?;
        let matches = actual.0 == expected.sha256 && actual.1 == expected.byte_size;
        if !matches && allow_missing {
            return Err(error("WEBDAV_REMOTE_CONFLICT"));
        }
        Ok(matches)
    }
    async fn put(
        &self,
        account: &WebDavAccount,
        url: &Url,
        expected: &Descriptor,
        body: reqwest::Body,
        cancel: &CancelFlag,
    ) -> Result<(), String> {
        if self.existing(account, url, expected, true, cancel).await? {
            return Ok(());
        }
        let result = self
            .request(
                account,
                url.clone(),
                Method::PUT,
                Some(body),
                Some(expected.byte_size),
                None,
                cancel,
                true,
            )
            .await;
        let uncertain = match result {
            Err(ref err) if err == "WEBDAV_NETWORK" => true,
            Err(err) => return Err(err),
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    false
                } else if matches!(status.as_u16(), 408 | 409 | 412 | 500..=599) {
                    true
                } else {
                    good(&response)?;
                    false
                }
            }
        };
        if !self.existing(account, url, expected, false, cancel).await? {
            return Err(error(if uncertain {
                "WEBDAV_REMOTE_CONFLICT"
            } else {
                "WEBDAV_REMOTE_INVALID"
            }));
        }
        Ok(())
    }
    async fn put_bytes(
        &self,
        account: &WebDavAccount,
        url: &Url,
        bytes: Vec<u8>,
        file: &str,
        cancel: &CancelFlag,
    ) -> Result<Descriptor, String> {
        let descriptor = Descriptor {
            file: file.into(),
            sha256: sha(&bytes),
            byte_size: bytes.len() as u64,
        };
        self.put(account, url, &descriptor, bytes.into(), cancel)
            .await?;
        Ok(descriptor)
    }
    pub async fn append(&self, input: AppendInput<'_>) -> Result<WebDavGeneration, String> {
        let endpoint = input.account.endpoint()?;
        if !valid_account_id(input.origin_project_id)
            || !safe_id(input.snapshot_generation)
            || !valid_account_id(input.generation_id)
            || !parents_valid(input.parents)
            || !timestamp(input.created_at)
        {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        let path = input.archive.to_path_buf();
        let maximum = self.max_archive_bytes;
        let flag = input.cancel.clone();
        let (hash, size, identity) =
            tokio::task::spawn_blocking(move || hash_file(&path, maximum, &flag))
                .await
                .map_err(|_| error("WEBDAV_INPUT_INVALID"))??;
        let archive = Descriptor {
            file: "archive.ainovel".into(),
            sha256: hash,
            byte_size: size,
        };
        let generation = Self::location(
            &endpoint,
            input.cloud_book_id,
            Some(input.generation_id),
            None,
        )?;
        let mut collections = Vec::new();
        let mut books = endpoint.clone();
        books
            .path_segments_mut()
            .map_err(|_| error("WEBDAV_INPUT_INVALID"))?
            .pop_if_empty()
            .push("books")
            .push("");
        collections.push(books.clone());
        let mut book = books;
        book.path_segments_mut()
            .map_err(|_| error("WEBDAV_INPUT_INVALID"))?
            .pop_if_empty()
            .push(input.cloud_book_id)
            .push("");
        collections.push(book);
        collections.push(Self::location(&endpoint, input.cloud_book_id, None, None)?);
        collections.push(generation.clone());
        for collection in collections {
            let response = self
                .request(
                    input.account,
                    collection,
                    method("MKCOL")?,
                    None,
                    None,
                    None,
                    input.cancel,
                    false,
                )
                .await?;
            if !matches!(response.status().as_u16(), 201 | 405) {
                good(&response)?;
                return Err(error("WEBDAV_NETWORK"));
            }
        }
        let mut parents = input.parents.to_vec();
        let mut seen = BTreeSet::new();
        parents.retain(|p| seen.insert(p.clone()));
        let manifest = Manifest {
            version: 1,
            cloud_book_id: input.cloud_book_id.into(),
            generation_id: input.generation_id.into(),
            parent_generation_ids: parents,
            created_at: input.created_at.into(),
            origin_project_id: input.origin_project_id.into(),
            portable_snapshot_generation: input.snapshot_generation.into(),
            archive: archive.clone(),
        };
        let handle =
            std::fs::File::open(input.archive).map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
        if files::handle_identity(&handle).map_err(|_| error("WEBDAV_INPUT_INVALID"))? != identity {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        let body = reqwest::Body::from(tokio::fs::File::from_std(handle));
        let archive_url = generation
            .join("archive.ainovel")
            .map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
        self.put(input.account, &archive_url, &archive, body, input.cancel)
            .await?;
        if files::path_identity(input.archive).map_err(|_| error("WEBDAV_INPUT_INVALID"))?
            != identity
        {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        let bytes = json_bytes(&manifest)?;
        let manifest_descriptor = self
            .put_bytes(
                input.account,
                &generation
                    .join("manifest.json")
                    .map_err(|_| error("WEBDAV_INPUT_INVALID"))?,
                bytes,
                "manifest.json",
                input.cancel,
            )
            .await?;
        let completion = Completion {
            version: 1,
            cloud_book_id: manifest.cloud_book_id.clone(),
            generation_id: manifest.generation_id.clone(),
            manifest: manifest_descriptor,
            archive,
            completed_at: input.created_at.into(),
        };
        self.put_bytes(
            input.account,
            &generation
                .join("completion.json")
                .map_err(|_| error("WEBDAV_INPUT_INVALID"))?,
            json_bytes(&completion)?,
            "completion.json",
            input.cancel,
        )
        .await?;
        Ok(public_generation(&manifest, vec![]))
    }
}

impl WebDavService {
    async fn inspect(
        &self,
        account: &WebDavAccount,
        book: &str,
        generation: &str,
        cancel: &CancelFlag,
        full: bool,
    ) -> Result<Manifest, String> {
        let endpoint = account.endpoint()?;
        let base = Self::location(&endpoint, book, Some(generation), None)?;
        let response = self
            .request(
                account,
                base.join("completion.json")
                    .map_err(|_| error("WEBDAV_INPUT_INVALID"))?,
                Method::GET,
                None,
                None,
                None,
                cancel,
                false,
            )
            .await?;
        good(&response)?;
        let completion: Completion =
            serde_json::from_slice(&read_bytes(response, 1024 * 1024, cancel).await?)
                .map_err(|_| error("WEBDAV_REMOTE_INVALID"))?;
        if completion.version != 1
            || completion.cloud_book_id != book
            || completion.generation_id != generation
            || !timestamp(&completion.completed_at)
            || !valid_descriptor(&completion.manifest, "manifest.json")
            || !valid_descriptor(&completion.archive, "archive.ainovel")
            || completion.archive.byte_size > self.max_archive_bytes
        {
            return Err(error("WEBDAV_REMOTE_INVALID"));
        }
        let response = self
            .request(
                account,
                base.join("manifest.json")
                    .map_err(|_| error("WEBDAV_INPUT_INVALID"))?,
                Method::GET,
                None,
                None,
                None,
                cancel,
                false,
            )
            .await?;
        good(&response)?;
        let bytes = read_bytes(response, 1024 * 1024, cancel).await?;
        if bytes.len() as u64 != completion.manifest.byte_size
            || sha(&bytes) != completion.manifest.sha256
        {
            return Err(error("WEBDAV_REMOTE_INVALID"));
        }
        let manifest: Manifest =
            serde_json::from_slice(&bytes).map_err(|_| error("WEBDAV_REMOTE_INVALID"))?;
        if manifest.version != 1
            || manifest.cloud_book_id != book
            || manifest.generation_id != generation
            || !valid_account_id(&manifest.origin_project_id)
            || !safe_id(&manifest.portable_snapshot_generation)
            || !timestamp(&manifest.created_at)
            || !parents_valid(&manifest.parent_generation_ids)
            || manifest
                .parent_generation_ids
                .iter()
                .collect::<BTreeSet<_>>()
                .len()
                != manifest.parent_generation_ids.len()
            || manifest.archive != completion.archive
        {
            return Err(error("WEBDAV_REMOTE_INVALID"));
        }
        let response = self
            .request(
                account,
                base.join("archive.ainovel")
                    .map_err(|_| error("WEBDAV_INPUT_INVALID"))?,
                if full { Method::GET } else { Method::HEAD },
                None,
                None,
                None,
                cancel,
                full,
            )
            .await?;
        good(&response)?;
        if full {
            let (hash, size) = hash_response(
                response,
                manifest
                    .archive
                    .byte_size
                    .max(1)
                    .min(self.max_archive_bytes),
                cancel,
            )
            .await?;
            if size != manifest.archive.byte_size || hash != manifest.archive.sha256 {
                return Err(error("WEBDAV_REMOTE_INVALID"));
            }
        } else if length(&response)? != Some(manifest.archive.byte_size) {
            return Err(error("WEBDAV_REMOTE_INVALID"));
        }
        Ok(manifest)
    }
    pub async fn list(
        &self,
        account: &WebDavAccount,
        book: &str,
        cancel: &CancelFlag,
    ) -> Result<Vec<WebDavGeneration>, String> {
        let generations = Self::location(&account.endpoint()?, book, None, None)?;
        let response = self
            .request(
                account,
                generations.clone(),
                method("PROPFIND")?,
                None,
                None,
                Some("1"),
                cancel,
                false,
            )
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(vec![]);
        }
        good(&response)?;
        if response.status() != StatusCode::MULTI_STATUS {
            return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
        }
        let bytes = read_bytes(response, 1024 * 1024, cancel).await?;
        let xml = std::str::from_utf8(&bytes).map_err(|_| error("WEBDAV_DAV_RESPONSE_INVALID"))?;
        let ids = generation_hrefs(xml, &generations)?;
        let mut manifests = Vec::new();
        for id in ids {
            match self.inspect(account, book, &id, cancel, true).await {
                Ok(manifest) => manifests.push(manifest),
                Err(err) if err == "WEBDAV_NOT_FOUND" || err == "WEBDAV_REMOTE_INVALID" => {}
                Err(err) => return Err(err),
            }
        }
        let key = |m: &Manifest| {
            let mut parents = m.parent_generation_ids.clone();
            parents.sort();
            parents
        };
        let mut groups: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
        for manifest in &manifests {
            groups
                .entry(key(manifest))
                .or_default()
                .push(manifest.generation_id.clone());
        }
        Ok(manifests
            .iter()
            .map(|m| {
                let siblings = groups[&key(m)]
                    .iter()
                    .filter(|id| **id != m.generation_id)
                    .cloned()
                    .collect();
                public_generation(m, siblings)
            })
            .collect())
    }
    /// 下载先写自有私有暂存，核大小与哈希后无覆盖发布；失败按创建身份清理。
    pub async fn download(
        &self,
        account: &WebDavAccount,
        book: &str,
        generation: &str,
        target: &Path,
        cancel: &CancelFlag,
    ) -> Result<(), String> {
        let target = files::absolute(target).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
        let parent = target
            .parent()
            .ok_or_else(|| error("WEBDAV_TARGET_UNSAFE"))?;
        files::safe_directory(parent).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
        if std::fs::symlink_metadata(&target).is_ok() {
            return Err(error("WEBDAV_TARGET_EXISTS"));
        }
        let manifest = self
            .inspect(account, book, generation, cancel, false)
            .await?;
        let mut attempt =
            files::prepare_staging(parent).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
        let result = async {
            let temporary = files::target_for(&mut attempt, "archive.ainovel")
                .map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
            let mut options = std::fs::OpenOptions::new();
            options.create_new(true).read(true).write(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let file = options
                .open(&temporary)
                .map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
            let identity =
                files::handle_identity(&file).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
            attempt.record_file(&temporary, identity);
            let mut file = tokio::fs::File::from_std(file);
            let url = Self::location(
                &account.endpoint()?,
                book,
                Some(generation),
                Some("archive.ainovel"),
            )?;
            let mut response = self
                .request(account, url, Method::GET, None, None, None, cancel, true)
                .await?;
            good(&response)?;
            if length(&response)?.is_some_and(|n| n != manifest.archive.byte_size) {
                return Err(error("WEBDAV_REMOTE_INVALID"));
            }
            let mut hasher = Sha256::new();
            let mut size = 0u64;
            while let Some(chunk) = checked(cancel, response.chunk()).await? {
                size = size
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| error("WEBDAV_REMOTE_INVALID"))?;
                if size > manifest.archive.byte_size || size > self.max_archive_bytes {
                    return Err(error("WEBDAV_REMOTE_INVALID"));
                }
                hasher.update(&chunk);
                file.write_all(&chunk)
                    .await
                    .map_err(|_| error("WEBDAV_PUBLISH_FAILED"))?;
            }
            if size != manifest.archive.byte_size
                || format!("{:x}", hasher.finalize()) != manifest.archive.sha256
            {
                return Err(error("WEBDAV_REMOTE_INVALID"));
            }
            file.sync_all()
                .await
                .map_err(|_| error("WEBDAV_PUBLISH_FAILED"))?;
            let file = file.into_std().await;
            if !files::same_object(
                &identity,
                &files::handle_identity(&file).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?,
            ) || !files::same_object(
                &identity,
                &files::path_identity(&temporary).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?,
            ) {
                return Err(error("WEBDAV_TARGET_UNSAFE"));
            }
            drop(file);
            files::assert_attempt_directory(&attempt, &attempt.root)
                .map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
            files::safe_directory(parent).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?;
            check_cancel(cancel)?;
            std::fs::hard_link(&temporary, &target).map_err(|e| {
                error(if e.kind() == std::io::ErrorKind::AlreadyExists {
                    "WEBDAV_TARGET_EXISTS"
                } else {
                    "WEBDAV_PUBLISH_FAILED"
                })
            })?;
            if !files::same_object(
                &identity,
                &files::path_identity(&target).map_err(|_| error("WEBDAV_TARGET_UNSAFE"))?,
            ) {
                return Err(error("WEBDAV_TARGET_UNSAFE"));
            }
            Ok(())
        }
        .await;
        files::cleanup_attempt(&attempt);
        result
    }
}
fn percent_decode(value: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut chars = value.bytes();
    while let Some(byte) = chars.next() {
        if byte == b'%' {
            let a = chars
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(|| error("WEBDAV_DAV_RESPONSE_INVALID"))?;
            let b = chars
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(|| error("WEBDAV_DAV_RESPONSE_INVALID"))?;
            bytes.push((a * 16 + b) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| error("WEBDAV_DAV_RESPONSE_INVALID"))
}
fn decode_entities(value: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        rest = &rest[index + 1..];
        let end = rest
            .find(';')
            .ok_or_else(|| error("WEBDAV_DAV_RESPONSE_INVALID"))?;
        let entity = &rest[..end];
        rest = &rest[end + 1..];
        let character = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(hex) = entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                {
                    u32::from_str_radix(hex, 16).ok()
                } else {
                    entity.strip_prefix('#').and_then(|n| n.parse::<u32>().ok())
                };
                code.and_then(char::from_u32)
                    .filter(|c| *c != '\0')
                    .ok_or_else(|| error("WEBDAV_DAV_RESPONSE_INVALID"))?
            }
        };
        output.push(character);
    }
    output.push_str(rest);
    Ok(output)
}
/// 与基线相同的小型DAV响应提取器，不解析DTD、实体扩展或CDATA；所有href逐个验证同源和路径。
fn generation_hrefs(xml: &str, root: &Url) -> Result<Vec<String>, String> {
    use std::sync::LazyLock;
    static FORBIDDEN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)<!DOCTYPE|<!ENTITY|<!\[CDATA\[|<![^-]").unwrap());
    static RESPONSE: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"(?is)<(?:[A-Za-z_][\w.-]*:)?response\b[^>]*>(.*?)</(?:[A-Za-z_][\w.-]*:)?response\s*>").unwrap()
    });
    static OPEN: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)<(?:[A-Za-z_][\w.-]*:)?response\b").unwrap());
    static HREF: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?is)<(?:[A-Za-z_][\w.-]*:)?href\b[^>]*>([^<]*)</(?:[A-Za-z_][\w.-]*:)?href\s*>",
        )
        .unwrap()
    });
    if FORBIDDEN.is_match(xml) {
        return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
    }
    let responses: Vec<_> = RESPONSE.captures_iter(xml).collect();
    if responses.is_empty() || responses.len() != OPEN.find_iter(xml).count() {
        return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
    }
    let mut root_path = percent_decode(root.path())?;
    if !root_path.ends_with('/') {
        root_path.push('/');
    }
    let mut generations = BTreeSet::new();
    for response in responses {
        let hrefs: Vec<_> = HREF.captures_iter(&response[1]).collect();
        if hrefs.len() != 1 {
            return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
        }
        let href = decode_entities(hrefs[0][1].trim())?;
        if href.is_empty() || href.starts_with("//") {
            return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
        }
        let url = root
            .join(&href)
            .map_err(|_| error("WEBDAV_DAV_RESPONSE_INVALID"))?;
        if url.origin() != root.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
        }
        let path = percent_decode(url.path())?;
        if path.chars().any(char::is_control) || path.contains('\\') {
            return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
        }
        if path == root_path || path == root_path.trim_end_matches('/') {
            continue;
        }
        let relative = path
            .strip_prefix(&root_path)
            .ok_or_else(|| error("WEBDAV_DAV_RESPONSE_INVALID"))?;
        let relative = relative.strip_suffix('/').unwrap_or(relative);
        if !valid_account_id(relative) || relative.contains('/') {
            return Err(error("WEBDAV_DAV_RESPONSE_INVALID"));
        }
        generations.insert(relative.to_string());
    }
    Ok(generations.into_iter().collect())
}

pub struct AppendInput<'a> {
    pub account: &'a WebDavAccount,
    pub cloud_book_id: &'a str,
    pub archive: &'a Path,
    pub origin_project_id: &'a str,
    pub snapshot_generation: &'a str,
    pub parents: &'a [String],
    pub generation_id: &'a str,
    pub created_at: &'a str,
    pub cancel: &'a CancelFlag,
}
fn method(value: &str) -> Result<Method, String> {
    Method::from_bytes(value.as_bytes()).map_err(|_| error("WEBDAV_INPUT_INVALID"))
}
fn good(response: &Response) -> Result<(), String> {
    match response.status().as_u16() {
        200..=299 => Ok(()),
        401 | 403 => Err(error("WEBDAV_AUTH")),
        404 => Err(error("WEBDAV_NOT_FOUND")),
        _ => Err(error("WEBDAV_NETWORK")),
    }
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn timestamp(value: &str) -> bool {
    super::portable_project_format::is_exact_iso8601_millis(value)
}
fn parents_valid(values: &[String]) -> bool {
    values.len() <= 256 && values.iter().all(|v| safe_id(v))
}
fn valid_descriptor(value: &Descriptor, file: &str) -> bool {
    value.file == file
        && value.sha256.len() == 64
        && value
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.byte_size <= ((1u64 << 53) - 1)
}
fn json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(value).map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(error("WEBDAV_INPUT_INVALID"));
    }
    Ok(bytes)
}
fn length(response: &Response) -> Result<Option<u64>, String> {
    response
        .headers()
        .get("content-length")
        .map(|value| {
            let text = value.to_str().map_err(|_| error("WEBDAV_REMOTE_INVALID"))?;
            if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
                return Err(error("WEBDAV_REMOTE_INVALID"));
            }
            text.parse().map_err(|_| error("WEBDAV_REMOTE_INVALID"))
        })
        .transpose()
}
async fn read_bytes(
    mut response: Response,
    maximum: u64,
    cancel: &CancelFlag,
) -> Result<Vec<u8>, String> {
    let declared = length(&response)?;
    if declared.is_some_and(|len| len > maximum) {
        return Err(error("WEBDAV_RESPONSE_TOO_LARGE"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = checked(cancel, response.chunk()).await? {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > maximum {
            return Err(error("WEBDAV_RESPONSE_TOO_LARGE"));
        }
        bytes.extend_from_slice(&chunk);
    }
    if declared.is_some_and(|n| n != bytes.len() as u64) {
        return Err(error("WEBDAV_REMOTE_INVALID"));
    }
    Ok(bytes)
}
async fn hash_response(
    mut response: Response,
    maximum: u64,
    cancel: &CancelFlag,
) -> Result<(String, u64), String> {
    let declared = length(&response)?;
    if declared.is_some_and(|len| len > maximum) {
        return Err(error("WEBDAV_RESPONSE_TOO_LARGE"));
    }
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    while let Some(chunk) = checked(cancel, response.chunk()).await? {
        size = size
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| error("WEBDAV_RESPONSE_TOO_LARGE"))?;
        if size > maximum {
            return Err(error("WEBDAV_RESPONSE_TOO_LARGE"));
        }
        hasher.update(&chunk);
    }
    if declared.is_some_and(|n| n != size) {
        return Err(error("WEBDAV_REMOTE_INVALID"));
    }
    Ok((format!("{:x}", hasher.finalize()), size))
}
fn hash_file(
    path: &Path,
    maximum: u64,
    cancel: &CancelFlag,
) -> Result<(String, u64, files::FileIdentity), String> {
    use std::io::Read;
    let before = files::path_identity(path).map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
    if !before.is_file
        || before.is_symlink
        || before.size > maximum
        || files::real_path_text(path) != files::literal_path_text(path)
    {
        return Err(error("WEBDAV_INPUT_INVALID"));
    }
    let mut file = std::fs::File::open(path).map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
    if files::handle_identity(&file).map_err(|_| error("WEBDAV_INPUT_INVALID"))? != before {
        return Err(error("WEBDAV_INPUT_INVALID"));
    }
    let mut hasher = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    let mut size = 0;
    loop {
        check_cancel(cancel)?;
        let n = file
            .read(&mut bytes)
            .map_err(|_| error("WEBDAV_INPUT_INVALID"))?;
        if n == 0 {
            break;
        }
        size += n as u64;
        if size > maximum {
            return Err(error("WEBDAV_INPUT_INVALID"));
        }
        hasher.update(&bytes[..n]);
    }
    if files::path_identity(path).map_err(|_| error("WEBDAV_INPUT_INVALID"))? != before
        || files::handle_identity(&file).map_err(|_| error("WEBDAV_INPUT_INVALID"))? != before
        || size != before.size
    {
        return Err(error("WEBDAV_INPUT_INVALID"));
    }
    Ok((format!("{:x}", hasher.finalize()), size, before))
}
#[cfg(test)]
#[path = "webdav_backup_tests.rs"]
mod tests;

fn public_generation(manifest: &Manifest, siblings: Vec<String>) -> WebDavGeneration {
    WebDavGeneration {
        cloud_book_id: manifest.cloud_book_id.clone(),
        generation_id: manifest.generation_id.clone(),
        parent_generation_ids: manifest.parent_generation_ids.clone(),
        created_at: manifest.created_at.clone(),
        origin_project_id: manifest.origin_project_id.clone(),
        portable_snapshot_generation: manifest.portable_snapshot_generation.clone(),
        archive_sha256: manifest.archive.sha256.clone(),
        archive_byte_size: manifest.archive.byte_size,
        has_sibling: !siblings.is_empty(),
        sibling_generation_ids: siblings,
    }
}
