//! 设置磁盘写入：独立线程负责临时文件、fsync 和替换，调用方在设置数据锁外等待结果。
//! 所有设置变更共用一把串行闸门；闸门不保护内存状态，只防止快捷键事务与磁盘快照交错。
use super::Settings;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, MutexGuard},
};

/// 一次完整配置快照；在写线程内序列化，调用方不持设置锁等待磁盘。
struct WriteJob {
    path: PathBuf,
    value: Settings,
    reply: mpsc::SyncSender<Result<(), String>>,
}

pub(crate) struct SettingsWriter {
    jobs: mpsc::Sender<WriteJob>,
    gate: Mutex<()>,
}

impl SettingsWriter {
    /// 写线程随进程退出；通道关闭后循环自然结束，不额外持有 AppHandle 或设置状态。
    pub(crate) fn new() -> Result<Self, String> {
        let (jobs, receiver) = mpsc::channel::<WriteJob>();
        std::thread::Builder::new()
            .name("settings-writer".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    let result = persist(&job.path, &job.value);
                    // 调用方可能已超时/离开；发送失败只表示没人再等结果。
                    let _ = job.reply.send(result);
                }
            })
            .map_err(|e| format!("无法启动设置写入线程：{e}"))?;
        Ok(Self {
            jobs,
            gate: Mutex::new(()),
        })
    }

    /// 快捷键注册/释放可能调度主线程；串行化整笔事务，避免两个保存互相回滚。
    /// 持有此闸门时不得再获取设置数据锁等待磁盘，否则会重新把 I/O 带回锁内。
    pub(crate) fn gate(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.gate.lock().map_err(|e| e.to_string())
    }

    /// 提交快照并等待写线程完成；只用于设置变更路径，不持有 `SettingsState.0`。
    pub(crate) fn persist(&self, path: &Path, value: &Settings) -> Result<(), String> {
        let (reply, wait) = mpsc::sync_channel(1);
        self.jobs
            .send(WriteJob {
                path: path.to_path_buf(),
                value: value.clone(),
                reply,
            })
            .map_err(|_| "设置写入线程已停止。".to_string())?;
        wait.recv()
            .map_err(|_| "设置写入线程已停止。".to_string())?
    }
}

/// 先完整写入同目录临时文件，再替换正式文件，避免中途失败留下半份 JSON。
/// 该函数只在写线程或单元测试中直接调用；命令路径应使用 `SettingsWriter::persist`。
pub(crate) fn persist(path: &Path, value: &Settings) -> Result<(), String> {
    let parent = path.parent().ok_or("配置路径无效。")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_writes_are_serialized_and_persist_complete_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let writer = SettingsWriter::new().unwrap();
        let mut first = Settings::default();
        first.delay_seconds = 3;
        let mut second = Settings::default();
        second.delay_seconds = 5;
        writer.persist(&path, &first).unwrap();
        writer.persist(&path, &second).unwrap();
        assert_eq!(super::super::load(&path), (second, None));
    }
}
