//! 云凭据存储，平移 cloud-credential-store.ts：严格文档、限额、仅元数据出 IPC。
//! 全局根仅由调用者传入 Lorekeeper 根；从不读 .vela 或项目目录。
//! 持久加密失败时仅会话保留并明确警告，损坏文件不得被默认值静默覆盖。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use super::cloud_secret_protector::{DpapiProtector, SecretProtector};

pub const SESSION_WARNING: &str = "CLOUD_CREDENTIAL_SESSION_ONLY";
const FILE_NAME: &str = "cloud-backup-credentials.json";
const MAX_SECRET_BYTES: usize = 64 * 1024;

fn invalid() -> String {
    "CLOUD_CREDENTIAL_STORE_INVALID".into()
}
fn input_invalid() -> String {
    "CLOUD_CREDENTIAL_INPUT_INVALID".into()
}

pub(crate) fn valid_account_id(value: &str) -> bool {
    static UUID: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
        )
        .expect("UUID 表达式合法")
    });
    UUID.is_match(value)
}
fn control(value: &str) -> bool {
    value
        .chars()
        .any(|c| c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c))
}
fn valid_username(value: &str) -> bool {
    !value.is_empty() && value.encode_utf16().count() <= 1024 && !control(value)
}
fn valid_secret(value: &[u8]) -> bool {
    !value.is_empty() && value.len() <= MAX_SECRET_BYTES
}

/// HTTPS；HTTP 仅允许三种字面回环主机。拒绝 URL 内凭据、查询、片段。
pub fn normalize_webdav_endpoint(value: &str) -> Result<String, String> {
    let candidate = value.trim();
    let error = || "CLOUD_ENDPOINT_INVALID".to_string();
    if candidate.is_empty()
        || candidate.encode_utf16().count() > 2048
        || control(candidate)
        || candidate.contains('?')
        || candidate.contains('#')
    {
        return Err(error());
    }
    let mut url = url::Url::parse(candidate).map_err(|_| error())?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
        || !(url.scheme() == "https" || url.scheme() == "http" && loopback)
    {
        return Err(error());
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url.to_string())
}

/// 仅元数据可序列化；不为明文凭据定义 Debug/Serialize。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudCredentialMetadata {
    pub account_id: String,
    pub endpoint: String,
    pub username: String,
    pub persistence: String,
}
#[derive(Serialize)]
pub struct CredentialSaveResult {
    pub metadata: CloudCredentialMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<&'static str>,
}
pub struct ResolvedCredential {
    pub metadata: CloudCredentialMetadata,
    pub secret: Option<String>,
    pub warning: Option<&'static str>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCredential {
    account_id: String,
    endpoint: String,
    username: String,
    encrypted_secret_base64: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    accounts: Vec<StoredCredential>,
}
struct SessionCredential {
    metadata: CloudCredentialMetadata,
    secret: String,
}

/// 应由 AppState 的互斥锁串行访问，避免本进程对账户文档的并发读改写。
pub struct CloudCredentialStore<P = DpapiProtector> {
    path: PathBuf,
    protector: P,
    session: BTreeMap<String, SessionCredential>,
}
impl CloudCredentialStore<DpapiProtector> {
    pub fn new(root: &Path) -> Self {
        Self::with_protector(root, DpapiProtector)
    }
}
impl<P: SecretProtector> CloudCredentialStore<P> {
    pub fn with_protector(root: &Path, protector: P) -> Self {
        Self {
            path: root.join(FILE_NAME),
            protector,
            session: BTreeMap::new(),
        }
    }
    fn metadata(account: &StoredCredential) -> CloudCredentialMetadata {
        CloudCredentialMetadata {
            account_id: account.account_id.clone(),
            endpoint: account.endpoint.clone(),
            username: account.username.clone(),
            persistence: "os-backed".into(),
        }
    }
    fn context(account: &StoredCredential) -> Result<Vec<u8>, String> {
        serde_json::to_vec(&(
            "Lorekeeper/cloud-credential/v1",
            &account.account_id,
            &account.endpoint,
            &account.username,
        ))
        .map_err(|_| invalid())
    }
    fn load(&self) -> Result<Document, String> {
        if let Some(parent) = self.path.parent() {
            match std::fs::symlink_metadata(parent) {
                Ok(_) => crate::services::portable_project_archive::safe_directory(parent)
                    .map_err(|_| invalid())?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Document {
                        version: 1,
                        accounts: vec![],
                    })
                }
                Err(_) => return Err(invalid()),
            }
        }
        let bytes = crate::services::portable_runtime_freeze::regular_file(
            &self.path.to_string_lossy(),
            32 * 1024 * 1024,
            None,
        )
        .map_err(|_| invalid())?;
        let Some(bytes) = bytes else {
            return Ok(Document {
                version: 1,
                accounts: vec![],
            });
        };
        let document: Document = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if document.version != 1 || document.accounts.len() > 128 {
            return Err(invalid());
        }
        let mut ids = HashSet::new();
        for account in &document.accounts {
            if !valid_account_id(&account.account_id)
                || !ids.insert(account.account_id.clone())
                || !valid_username(&account.username)
                || normalize_webdav_endpoint(&account.endpoint).ok().as_deref()
                    != Some(&account.endpoint)
                || account.encrypted_secret_base64.is_empty()
                || account.encrypted_secret_base64.len() > MAX_SECRET_BYTES * 3
            {
                return Err(invalid());
            }
            let decoded =
                super::character_asset_service::base64_decode(&account.encrypted_secret_base64)
                    .map_err(|_| invalid())?;
            if super::character_asset_service::base64_encode(&decoded)
                != account.encrypted_secret_base64
            {
                return Err(invalid());
            }
        }
        Ok(document)
    }
    fn write(&self, document: &Document) -> Result<(), String> {
        let parent = self.path.parent().ok_or_else(invalid)?;
        std::fs::create_dir_all(parent).map_err(|_| invalid())?;
        crate::services::portable_project_archive::safe_directory(parent).map_err(|_| invalid())?;
        crate::json_store::write_json_file(
            &self.path,
            &serde_json::to_value(document).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())
    }
    pub fn list(&self) -> Result<Vec<CloudCredentialMetadata>, String> {
        let mut accounts: BTreeMap<String, CloudCredentialMetadata> = self
            .load()?
            .accounts
            .iter()
            .map(|account| (account.account_id.clone(), Self::metadata(account)))
            .collect();
        for (id, value) in &self.session {
            accounts.insert(id.clone(), value.metadata.clone());
        }
        Ok(accounts.into_values().collect())
    }
    pub fn save(
        &mut self,
        account_id: Option<&str>,
        endpoint: &str,
        username: &str,
        secret: &str,
    ) -> Result<CredentialSaveResult, String> {
        if !self.protector.supported() {
            return Err("CLOUD_CREDENTIAL_PLATFORM_UNSUPPORTED".into());
        }
        let endpoint = normalize_webdav_endpoint(endpoint)?;
        if !valid_username(username) || !valid_secret(secret.as_bytes()) {
            return Err(input_invalid());
        }
        let id = account_id
            .map(str::to_string)
            .unwrap_or_else(crate::project_access::random_uuid_v4);
        if !valid_account_id(&id) {
            return Err(input_invalid());
        }
        let mut doc = self.load()?;
        let existing = doc.accounts.iter().any(|account| account.account_id == id);
        if !existing
            && self.session.get(&id).is_none()
            && doc.accounts.len() + self.session.len() >= 128
        {
            return Err(input_invalid());
        }
        let mut stored = StoredCredential {
            account_id: id.clone(),
            endpoint,
            username: username.into(),
            encrypted_secret_base64: String::new(),
        };
        let encrypted = self
            .protector
            .encrypt(secret.as_bytes(), &Self::context(&stored)?)
            .ok()
            .filter(|bytes| !bytes.is_empty() && bytes.len() <= 128 * 1024);
        doc.accounts.retain(|account| account.account_id != id);
        if let Some(encrypted) = encrypted {
            stored.encrypted_secret_base64 =
                super::character_asset_service::base64_encode(&encrypted);
            let metadata = Self::metadata(&stored);
            doc.accounts.push(stored);
            self.write(&doc)?;
            self.session.remove(&id);
            Ok(CredentialSaveResult {
                metadata,
                warning: None,
            })
        } else {
            if existing {
                self.write(&doc)?;
            }
            let mut metadata = Self::metadata(&stored);
            metadata.persistence = "session-only".into();
            self.session.insert(
                id,
                SessionCredential {
                    metadata: metadata.clone(),
                    secret: secret.into(),
                },
            );
            Ok(CredentialSaveResult {
                metadata,
                warning: Some(SESSION_WARNING),
            })
        }
    }
    pub fn resolve(&self, account_id: &str) -> Result<Option<ResolvedCredential>, String> {
        if !self.protector.supported() {
            return Err("CLOUD_CREDENTIAL_PLATFORM_UNSUPPORTED".into());
        }
        if !valid_account_id(account_id) {
            return Err(input_invalid());
        }
        if let Some(session) = self.session.get(account_id) {
            return Ok(Some(ResolvedCredential {
                metadata: session.metadata.clone(),
                secret: Some(session.secret.clone()),
                warning: Some(SESSION_WARNING),
            }));
        }
        let Some(account) = self
            .load()?
            .accounts
            .into_iter()
            .find(|account| account.account_id == account_id)
        else {
            return Ok(None);
        };
        let cipher =
            super::character_asset_service::base64_decode(&account.encrypted_secret_base64)
                .map_err(|_| invalid())?;
        let secret = self
            .protector
            .decrypt(&cipher, &Self::context(&account)?)
            .ok()
            .filter(|plain| valid_secret(plain))
            .and_then(|plain| String::from_utf8(plain).ok());
        let warning = if secret.is_none() {
            Some(SESSION_WARNING)
        } else {
            None
        };
        Ok(Some(ResolvedCredential {
            metadata: Self::metadata(&account),
            secret,
            warning,
        }))
    }
    pub fn clear(&mut self, account_id: &str) -> Result<bool, String> {
        if !valid_account_id(account_id) {
            return Err(input_invalid());
        }
        let mut doc = self.load()?;
        let before = doc.accounts.len();
        doc.accounts
            .retain(|account| account.account_id != account_id);
        let removed = before != doc.accounts.len();
        if removed {
            self.write(&doc)?;
        }
        Ok(self.session.remove(account_id).is_some() || removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "lorekeeper-cloud-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir_all(&p).unwrap();
        std::fs::canonicalize(p).unwrap()
    }
    #[test]
    fn endpoints_reject_plaintext_remote_credentials_and_fragments() {
        assert_eq!(
            normalize_webdav_endpoint(" https://example.test/dav ").unwrap(),
            "https://example.test/dav/"
        );
        assert!(normalize_webdav_endpoint("http://localhost:1234/dav").is_ok());
        for endpoint in [
            "http://remote.test/",
            "https://user:password@example.test/",
            "https://example.test/?q=x",
            "https://example.test/#fragment",
            "file:///tmp",
            "https://example.test/\nsecret",
        ] {
            assert!(normalize_webdav_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }
    struct Unavailable;
    impl SecretProtector for Unavailable {
        fn encrypt(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>, String> {
            Err("不可用".into())
        }
        fn decrypt(&self, _: &[u8], _: &[u8]) -> Result<Vec<u8>, String> {
            Err("不可用".into())
        }
    }
    #[test]
    fn unavailable_encryption_keeps_only_session_secret_and_rejects_corrupt_file() {
        let root = root();
        let mut store = CloudCredentialStore::with_protector(&root, Unavailable);
        let result = store
            .save(
                None,
                "https://example.test/",
                "test-user",
                "unit-test-only-secret",
            )
            .unwrap();
        assert_eq!(result.warning, Some(SESSION_WARNING));
        assert_eq!(
            store
                .resolve(&result.metadata.account_id)
                .unwrap()
                .unwrap()
                .secret
                .as_deref(),
            Some("unit-test-only-secret")
        );
        assert!(!root.join(FILE_NAME).exists());
        assert!(store.clear(&result.metadata.account_id).unwrap());
        std::fs::write(root.join(FILE_NAME), b"corrupt").unwrap();
        assert!(store
            .save(None, "https://example.test/", "test-user", "secret")
            .is_err());
        assert_eq!(std::fs::read(root.join(FILE_NAME)).unwrap(), b"corrupt");
        let _ = std::fs::remove_dir_all(root);
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_store_roundtrips_without_plaintext_and_binds_metadata() {
        let root = root();
        let mut store = CloudCredentialStore::new(&root);
        let result = store
            .save(
                None,
                "https://example.test/dav",
                "test-user",
                "unit-test-only-secret",
            )
            .unwrap();
        assert!(result.warning.is_none());
        let bytes = std::fs::read(root.join(FILE_NAME)).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("unit-test-only-secret"));
        let id = result.metadata.account_id;
        drop(store);
        let mut store = CloudCredentialStore::new(&root);
        assert_eq!(
            store.resolve(&id).unwrap().unwrap().secret.as_deref(),
            Some("unit-test-only-secret")
        );
        let mut document: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        document["accounts"][0]["endpoint"] = serde_json::json!("https://other.test/");
        std::fs::write(root.join(FILE_NAME), serde_json::to_vec(&document).unwrap()).unwrap();
        let unavailable = store.resolve(&id).unwrap().unwrap();
        assert!(unavailable.secret.is_none());
        assert_eq!(unavailable.warning, Some(SESSION_WARNING));
        assert!(store.clear(&id).unwrap());
        assert!(store.list().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }
}
