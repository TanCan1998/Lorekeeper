//! 按项目存储根串行化知识库读取副本与规范数据修改。
//! 非可重入：只在最外层编排获取，内部仓储与向量帮助函数不重复获取。
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub async fn acquire(project_root: &str) -> Result<OwnedMutexGuard<()>, String> {
    static GATES: OnceLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let key = crate::security::normalized_project_path(project_root);
    let gate = {
        let mut gates = GATES
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| "知识库串行锁状态被污染".to_string())?;
        gates.retain(|_, gate| gate.strong_count() > 0);
        match gates.get(&key).and_then(Weak::upgrade) {
            Some(gate) => gate,
            None => {
                let gate = Arc::new(AsyncMutex::new(()));
                gates.insert(key, Arc::downgrade(&gate));
                gate
            }
        }
    };
    Ok(gate.lock_owned().await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_project_waits_and_other_project_is_independent() {
        let root = format!("gate-{}", crate::project_access::random_uuid_v4());
        let held = acquire(&root).await.unwrap();
        let next = acquire(&root);
        tokio::pin!(next);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), &mut next)
                .await
                .is_err()
        );
        let other = acquire(&format!("{root}-other")).await.unwrap();
        drop(other);
        drop(held);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), &mut next)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn cancelled_waiter_does_not_strand_the_gate() {
        let root = format!("gate-{}", crate::project_access::random_uuid_v4());
        let held = acquire(&root).await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), acquire(&root),)
                .await
                .is_err()
        );
        drop(held);
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), acquire(&root),)
                .await
                .is_ok()
        );
    }
}
