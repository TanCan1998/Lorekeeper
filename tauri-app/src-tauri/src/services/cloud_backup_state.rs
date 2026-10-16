//! 云备份进程状态：本地元数据单锁串行，网络操作只持有独立取消标志，绝不跨await持锁。
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::cloud_credential_store::CloudCredentialStore;
use super::cloud_project_binding_store::CloudProjectBindingStore;
use super::webdav_backup::{cancellation, CancelFlag};

pub struct CloudLocalState {
    pub credentials: CloudCredentialStore,
    pub bindings: CloudProjectBindingStore,
}
pub struct CloudBackupState {
    pub root: PathBuf,
    local: Mutex<CloudLocalState>,
    operations: Mutex<HashMap<String, CancelFlag>>,
}
impl std::fmt::Debug for CloudBackupState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudBackupState").finish_non_exhaustive()
    }
}
impl CloudBackupState {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            local: Mutex::new(CloudLocalState {
                credentials: CloudCredentialStore::new(root),
                bindings: CloudProjectBindingStore::new(root),
            }),
            operations: Mutex::new(HashMap::new()),
        }
    }
    pub fn local(&self) -> Result<MutexGuard<'_, CloudLocalState>, String> {
        self.local
            .lock()
            .map_err(|_| "CLOUD_BACKUP_LOCAL_STATE_NOT_SAVED".into())
    }
    pub fn begin(&self, id: &str) -> Result<Operation<'_>, String> {
        if !super::cloud_credential_store::valid_account_id(id) {
            return Err("CLOUD_BACKUP_INPUT_INVALID".into());
        }
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "CLOUD_BACKUP_FAILED".to_string())?;
        if operations.contains_key(id) {
            return Err("CLOUD_BACKUP_OPERATION_CONFLICT".into());
        }
        let cancel = cancellation();
        operations.insert(id.to_string(), cancel.clone());
        Ok(Operation {
            owner: self,
            id: id.into(),
            cancel,
        })
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        if !super::cloud_credential_store::valid_account_id(id) {
            return Err("CLOUD_BACKUP_INPUT_INVALID".into());
        }
        let operations = self
            .operations
            .lock()
            .map_err(|_| "CLOUD_BACKUP_FAILED".to_string())?;
        let flag = operations
            .get(id)
            .ok_or_else(|| "CLOUD_BACKUP_OPERATION_NOT_FOUND".to_string())?;
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
pub struct Operation<'a> {
    owner: &'a CloudBackupState,
    pub id: String,
    pub cancel: CancelFlag,
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.owner.operations.lock() {
            operations.remove(&self.id);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn operation_identity_stays_reserved_until_owner_finishes() {
        let cloud = CloudBackupState::new(Path::new("not-used"));
        let id = "11111111-1111-4111-8111-111111111111";
        let operation = cloud.begin(id).unwrap();
        assert!(cloud.begin(id).is_err());
        cloud.cancel(id).unwrap();
        assert!(operation.cancel.load(std::sync::atomic::Ordering::SeqCst));
        assert!(cloud.begin(id).is_err());
        drop(operation);
        assert!(cloud.cancel(id).is_err());
        assert!(cloud.begin(id).is_ok());
    }
}
