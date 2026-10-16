//! 云项目绑定的全局元数据：乐观修订、恢复来源只读、清除凭据后失配。
//! 不写入项目库/归档，不复用上游全局根。调用者须以进程内互斥锁串行访问。
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::cloud_credential_store::valid_account_id;
use serde::{Deserialize, Serialize};

const MAX_REVISION: u64 = (1u64 << 53) - 1;

pub fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:@+-".contains(&byte))
}
fn invalid() -> String {
    "CLOUD_BINDING_INPUT_INVALID".into()
}
fn document_invalid() -> String {
    "CLOUD_BINDING_DOCUMENT_INVALID".into()
}
fn conflict() -> String {
    "CLOUD_BINDING_REVISION_CONFLICT".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BindingMode {
    Unconfigured,
    Writable,
    OriginReadonly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CloudProjectBinding {
    pub local_project_id: String,
    pub cloud_book_id: String,
    pub local_endpoint_account_id: String,
    pub last_selected_parent_generation_ids: Vec<String>,
    pub mode: BindingMode,
    pub revision: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    bindings: Vec<CloudProjectBinding>,
}

pub struct BindingInput<'a> {
    pub local_project_id: &'a str,
    pub cloud_book_id: &'a str,
    pub account_id: &'a str,
    pub parents: &'a [String],
}
fn parents_valid(parents: &[String]) -> bool {
    parents.len() <= 256 && parents.iter().all(|id| safe_id(id))
}
fn validate(binding: &CloudProjectBinding) -> bool {
    valid_account_id(&binding.local_project_id)
        && safe_id(&binding.cloud_book_id)
        && safe_id(&binding.local_endpoint_account_id)
        && parents_valid(&binding.last_selected_parent_generation_ids)
        && binding
            .last_selected_parent_generation_ids
            .iter()
            .collect::<HashSet<_>>()
            .len()
            == binding.last_selected_parent_generation_ids.len()
        && binding.revision > 0
        && binding.revision <= MAX_REVISION
}
fn next(revision: u64) -> Result<u64, String> {
    if revision >= MAX_REVISION {
        Err(conflict())
    } else {
        Ok(revision + 1)
    }
}
pub struct CloudProjectBindingStore {
    path: PathBuf,
}
impl CloudProjectBindingStore {
    pub fn new(root: &Path) -> Self {
        Self {
            path: root.join("cloud-backup-bindings.json"),
        }
    }
    fn read(&self) -> Result<Document, String> {
        if let Some(parent) = self.path.parent() {
            match std::fs::symlink_metadata(parent) {
                Ok(_) => super::portable_project_archive::safe_directory(parent)
                    .map_err(|_| document_invalid())?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Document {
                        version: 1,
                        bindings: vec![],
                    })
                }
                Err(_) => return Err(document_invalid()),
            }
        }
        let bytes = super::portable_runtime_freeze::regular_file(
            &self.path.to_string_lossy(),
            32 * 1024 * 1024,
            None,
        )
        .map_err(|_| document_invalid())?;
        let Some(bytes) = bytes else {
            return Ok(Document {
                version: 1,
                bindings: vec![],
            });
        };
        let doc: Document = serde_json::from_slice(&bytes).map_err(|_| document_invalid())?;
        let mut seen = HashSet::new();
        if doc.version != 1
            || doc.bindings.len() > 10_000
            || doc
                .bindings
                .iter()
                .any(|b| !validate(b) || !seen.insert(b.local_project_id.clone()))
        {
            return Err(document_invalid());
        }
        Ok(doc)
    }
    fn write(&self, doc: &Document) -> Result<(), String> {
        if doc.bindings.len() > 10_000 || doc.bindings.iter().any(|b| !validate(b)) {
            return Err(document_invalid());
        }
        let parent = self.path.parent().ok_or_else(document_invalid)?;
        std::fs::create_dir_all(parent).map_err(|_| document_invalid())?;
        super::portable_project_archive::safe_directory(parent).map_err(|_| document_invalid())?;
        crate::json_store::write_json_file(
            &self.path,
            &serde_json::to_value(doc).map_err(|_| document_invalid())?,
        )
        .map_err(|_| document_invalid())
    }
    pub fn list(&self) -> Result<Vec<CloudProjectBinding>, String> {
        Ok(self.read()?.bindings)
    }
    pub fn get(&self, id: &str) -> Result<Option<CloudProjectBinding>, String> {
        if !valid_account_id(id) {
            return Err(invalid());
        }
        Ok(self
            .read()?
            .bindings
            .into_iter()
            .find(|b| b.local_project_id == id))
    }
    fn normalized(
        input: BindingInput<'_>,
        mode: BindingMode,
        revision: u64,
    ) -> Result<CloudProjectBinding, String> {
        if !valid_account_id(input.local_project_id)
            || !safe_id(input.cloud_book_id)
            || !safe_id(input.account_id)
            || !parents_valid(input.parents)
        {
            return Err(invalid());
        }
        let mut seen = HashSet::new();
        Ok(CloudProjectBinding {
            local_project_id: input.local_project_id.into(),
            cloud_book_id: input.cloud_book_id.into(),
            local_endpoint_account_id: input.account_id.into(),
            last_selected_parent_generation_ids: input
                .parents
                .iter()
                .filter(|id| seen.insert((*id).clone()))
                .cloned()
                .collect(),
            mode,
            revision,
        })
    }
    pub fn save_writable(
        &mut self,
        input: BindingInput<'_>,
        expected_revision: Option<u64>,
    ) -> Result<CloudProjectBinding, String> {
        if expected_revision.is_some_and(|r| r == 0 || r > MAX_REVISION) {
            return Err(invalid());
        }
        let mut saved = Self::normalized(input, BindingMode::Writable, 1)?;
        let mut doc = self.read()?;
        let index = doc
            .bindings
            .iter()
            .position(|b| b.local_project_id == saved.local_project_id);
        let current = index.map(|i| doc.bindings[i].revision);
        if current != expected_revision {
            return Err(conflict());
        }
        if let Some(i) = index {
            saved.revision = next(doc.bindings[i].revision)?;
            doc.bindings[i] = saved.clone();
        } else {
            doc.bindings.push(saved.clone());
        }
        self.write(&doc)?;
        Ok(saved)
    }
    pub fn save_origin_readonly(
        &mut self,
        input: BindingInput<'_>,
    ) -> Result<CloudProjectBinding, String> {
        let saved = Self::normalized(input, BindingMode::OriginReadonly, 1)?;
        let mut doc = self.read()?;
        if let Some(current) = doc
            .bindings
            .iter()
            .find(|b| b.local_project_id == saved.local_project_id)
        {
            if current.mode == saved.mode
                && current.cloud_book_id == saved.cloud_book_id
                && current.local_endpoint_account_id == saved.local_endpoint_account_id
                && current.last_selected_parent_generation_ids
                    == saved.last_selected_parent_generation_ids
            {
                return Ok(current.clone());
            }
            return Err(conflict());
        }
        doc.bindings.push(saved.clone());
        self.write(&doc)?;
        Ok(saved)
    }
    pub fn mark_account_unconfigured(&mut self, id: &str) -> Result<usize, String> {
        if !safe_id(id) {
            return Err(invalid());
        }
        let mut doc = self.read()?;
        let mut changed = 0;
        for binding in &mut doc.bindings {
            if binding.local_endpoint_account_id == id && binding.mode != BindingMode::Unconfigured
            {
                binding.mode = BindingMode::Unconfigured;
                binding.revision = next(binding.revision)?;
                changed += 1;
            }
        }
        if changed > 0 {
            self.write(&doc)?;
        }
        Ok(changed)
    }
    pub fn remove_deleted_project(&mut self, id: &str) -> Result<bool, String> {
        if !valid_account_id(id) {
            return Err(invalid());
        }
        let mut doc = self.read()?;
        let count = doc.bindings.len();
        doc.bindings.retain(|b| b.local_project_id != id);
        if doc.bindings.len() == count {
            return Ok(false);
        }
        self.write(&doc)?;
        Ok(true)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    const PROJECT: &str = "123e4567-e89b-42d3-a456-426614174000";
    fn root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "lorekeeper-binding-{}",
            crate::project_access::random_uuid_v4()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::canonicalize(root).unwrap()
    }
    fn input(parents: &[String]) -> BindingInput<'_> {
        BindingInput {
            local_project_id: PROJECT,
            cloud_book_id: "book",
            account_id: "account",
            parents,
        }
    }
    #[test]
    fn revisions_and_readonly_restore_require_explicit_rebinding() {
        let root = root();
        let mut store = CloudProjectBindingStore::new(&root);
        let parents = vec!["parent".into(), "parent".into()];
        let saved = store.save_origin_readonly(input(&parents)).unwrap();
        assert_eq!(saved.mode, BindingMode::OriginReadonly);
        assert_eq!(saved.last_selected_parent_generation_ids.len(), 1);
        assert_eq!(store.save_origin_readonly(input(&parents)).unwrap(), saved);
        assert_eq!(
            store.save_writable(input(&parents), None).unwrap_err(),
            conflict()
        );
        let writable = store.save_writable(input(&parents), Some(1)).unwrap();
        assert_eq!(writable.revision, 2);
        assert_eq!(writable.mode, BindingMode::Writable);
        assert_eq!(
            store.save_writable(input(&parents), Some(1)).unwrap_err(),
            conflict()
        );
        assert_eq!(store.mark_account_unconfigured("account").unwrap(), 1);
        assert_eq!(store.get(PROJECT).unwrap().unwrap().revision, 3);
        assert_eq!(store.mark_account_unconfigured("account").unwrap(), 0);
        assert!(store.remove_deleted_project(PROJECT).unwrap());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn corrupt_or_overspecified_documents_are_not_overwritten() {
        let root = root();
        let mut store = CloudProjectBindingStore::new(&root);
        let raw = b"{\"version\":1,\"bindings\":[],\"extra\":true}";
        std::fs::write(&store.path, raw).unwrap();
        assert!(store.save_writable(input(&[]), None).is_err());
        assert_eq!(std::fs::read(&store.path).unwrap(), raw);
        let _ = std::fs::remove_dir_all(root);
    }
}
