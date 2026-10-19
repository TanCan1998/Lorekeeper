//! 头像读取缓存：项目/租约/数据根隔离，按资产修订索引，FIFO 上限 512。
use crate::repositories::character_asset_repository as repository;
use crate::security::ProjectSessionContext;
use crate::services::character_asset_service as avatar;
use rusqlite::Connection;
use std::collections::VecDeque;
use std::path::Path;

#[derive(Default)]
pub(crate) struct AvatarCache {
    scope: Option<(String, String, std::path::PathBuf)>,
    entries: VecDeque<avatar::CharacterAvatarView>,
}

/// 只打印作用域与条目数：头像内容是用户图片，不进日志与调试输出。
impl std::fmt::Debug for AvatarCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AvatarCache")
            .field("scoped", &self.scope.is_some())
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl AvatarCache {
    pub(crate) fn enter_scope(&mut self, context: &ProjectSessionContext, root: &Path) {
        let next = (
            context.project_id.clone(),
            context.lease_id.clone(),
            root.to_path_buf(),
        );
        if self.scope.as_ref() != Some(&next) {
            self.entries.clear();
            self.scope = Some(next);
        }
    }

    pub(crate) fn invalidate(&mut self, character_id: &str) {
        self.entries
            .retain(|item| item.character_id != character_id);
    }

    fn insert(&mut self, item: avatar::CharacterAvatarView) {
        self.entries.retain(|old| {
            old.character_id != item.character_id || old.asset_revision != item.asset_revision
        });
        self.entries.push_back(item);
        while self.entries.len() > 512 {
            self.entries.pop_front();
        }
    }

    pub(crate) fn read_many(
        &mut self,
        conn: &Connection,
        root: &Path,
        ids: &[String],
    ) -> Result<Vec<avatar::CharacterAvatarView>, String> {
        if ids.len() > 256 || ids.iter().any(|id| !avatar::is_valid_character_id(id)) {
            return Err("INVALID_CHARACTER_ID".to_string());
        }
        // 数据库记录始终重读；不存在的记录不允许被内存缓存“复活”。
        let records = repository::read_many(conn, ids)?;
        let mut result = Vec::new();
        let mut missing = Vec::new();
        for record in records {
            if let Some(item) = self.entries.iter().find(|item| {
                item.character_id == record.character_id
                    && item.asset_revision == record.asset_revision
            }) {
                result.push(item.clone());
            } else {
                missing.push(record.character_id);
            }
        }
        for item in avatar::read_many(conn, root, &missing)? {
            self.insert(item.clone());
            result.push(item);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(lease: &str) -> ProjectSessionContext {
        ProjectSessionContext {
            project_id: "synthetic-project".into(),
            lease_id: lease.into(),
            project_path: "synthetic-root".into(),
        }
    }
    fn view(id: &str, revision: i64) -> avatar::CharacterAvatarView {
        avatar::CharacterAvatarView {
            character_id: id.into(),
            asset_revision: revision,
            mime: "image/png".into(),
            base64: "c3ludGhldGlj".into(),
        }
    }
    #[test]
    fn cache_scope_changes_clear_content_test() {
        let mut cache = AvatarCache::default();
        cache.enter_scope(&context("lease-1"), Path::new("root-a"));
        cache.insert(view("char-a", 1));
        cache.enter_scope(&context("lease-1"), Path::new("root-a"));
        assert_eq!(cache.entries.len(), 1);
        cache.enter_scope(&context("lease-2"), Path::new("root-a"));
        assert!(cache.entries.is_empty());
        cache.insert(view("char-a", 1));
        cache.enter_scope(&context("lease-2"), Path::new("root-b"));
        assert!(cache.entries.is_empty());
        cache.insert(view("char-a", 1));
        let mut next = context("lease-2");
        next.project_id = "other-project".into();
        cache.enter_scope(&next, Path::new("root-b"));
        assert!(cache.entries.is_empty());
    }
    #[test]
    fn cache_invalidates_all_revisions_and_bounds_fifo_test() {
        let mut cache = AvatarCache::default();
        for i in 0..514 {
            cache.insert(view(&format!("char-{i}"), 1));
        }
        assert_eq!(cache.entries.len(), 512);
        assert_eq!(cache.entries.front().unwrap().character_id, "char-2");
        cache.insert(view("char-2", 2));
        cache.invalidate("char-2");
        assert!(cache
            .entries
            .iter()
            .all(|item| item.character_id != "char-2"));
    }
    #[test]
    fn cache_validates_raw_batch_before_any_database_read_test() {
        let conn = Connection::open_in_memory().unwrap();
        let mut cache = AvatarCache::default();
        for ids in [
            vec!["char-a".to_string(); 257],
            vec!["../unsafe".to_string()],
        ] {
            assert_eq!(
                cache.read_many(&conn, Path::new("unused"), &ids),
                Err("INVALID_CHARACTER_ID".into())
            );
        }
    }
}
