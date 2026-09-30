//! R8:<server> 服务端韧性 完成，待主控接线。
//!
//! - 全局并发 turn 上限：`ShutdownGate::try_acquire_turn`（信号量 + 活跃计数）；
//! - 优雅关闭：`request_shutdown`（停止接收）→ `await_drain`（完成在途）→ 主控 flush → 退出；
//! - 强杀恢复：`PidFile`（<data_root>/server.pid）+ `recover_force_kill`（陈旧 pid 清理）。
//!
//! 模块约定：不引用 `crate::`/`super::`，可被测试以 #[path] mod 独立编译。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 默认全局并发回合上限（环境变量 OWO_SERVER_MAX_CONCURRENT_TURNS 可调）。
pub const DEFAULT_MAX_CONCURRENT_TURNS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnBusy {
    AtCapacity,
    ShuttingDown,
}

impl std::fmt::Display for TurnBusy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TurnBusy::AtCapacity => {
                write!(formatter, "并发回合已达上限，请稍后重试")
            }
            TurnBusy::ShuttingDown => {
                write!(formatter, "服务正在优雅关闭，停止接收新回合")
            }
        }
    }
}

/// 优雅关闭结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GracefulOutcome {
    /// 请求关闭时的在途回合数。
    pub active_at_request: usize,
    /// 等待期限内完成的回合数（active_at_request - remaining）。
    pub drained: usize,
    /// 超时后仍在途的回合数（>0 表示需要强杀路径兜底）。
    pub remaining: usize,
}

pub struct ShutdownGate {
    semaphore: Arc<tokio::sync::Semaphore>,
    active: Arc<AtomicUsize>,
    shutting_down: Arc<AtomicBool>,
    request: Arc<tokio::sync::Notify>,
    max_concurrent: usize,
}

pub struct TurnPermit {
    _semaphore: tokio::sync::OwnedSemaphorePermit,
    active: Arc<AtomicUsize>,
}

impl std::fmt::Debug for TurnPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TurnPermit")
            .field("active", &self.active.load(Ordering::SeqCst))
            .finish()
    }
}

impl Drop for TurnPermit {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

impl ShutdownGate {
    /// 环境变量构造：OWO_SERVER_MAX_CONCURRENT_TURNS（默认 4，最小 1）。
    pub fn from_env() -> Self {
        let max = std::env::var("OWO_SERVER_MAX_CONCURRENT_TURNS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(DEFAULT_MAX_CONCURRENT_TURNS)
            .max(1);
        Self::new(max)
    }

    pub fn new(max_concurrent: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(max_concurrent.max(1))),
            active: Arc::new(AtomicUsize::new(0)),
            shutting_down: Arc::new(AtomicBool::new(false)),
            request: Arc::new(tokio::sync::Notify::new()),
            max_concurrent: max_concurrent.max(1),
        }
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    /// 当前在途回合数。
    pub fn active_turns(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    pub fn shutting_down(&self) -> bool {
        self.shutting_down.load(Ordering::Acquire)
    }

    /// 获取回合许可；上限已满或正在关闭时拒绝（不等待、不排队）。
    pub fn try_acquire_turn(&self) -> Result<TurnPermit, TurnBusy> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(TurnBusy::ShuttingDown);
        }
        let permit = self
            .semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| TurnBusy::AtCapacity)?;
        self.active.fetch_add(1, Ordering::SeqCst);
        Ok(TurnPermit {
            _semaphore: permit,
            active: Arc::clone(&self.active),
        })
    }

    /// 请求优雅关闭：停止接收新回合，返回当时在途数。
    pub fn request_shutdown(&self) -> usize {
        self.shutting_down.store(true, Ordering::Release);
        self.request.notify_waiters();
        self.active_turns()
    }

    /// 等待关闭请求（在途回合完成前的信号）。
    pub async fn wait_shutdown_request(&self) {
        self.request.notified().await;
    }

    /// 等待在途回合完成，超时返回剩余数（>0 由强杀恢复路径兜底）。
    pub async fn await_drain(&self, timeout: Duration) -> usize {
        let deadline = tokio::time::Instant::now() + timeout;
        while self.active_turns() > 0 {
            if tokio::time::Instant::now() >= deadline {
                return self.active_turns();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        0
    }

    /// 完整优雅关闭：停止接收 → 完成在途（限时）。
    pub async fn graceful_shutdown(&self, timeout: Duration) -> GracefulOutcome {
        let active_at_request = self.request_shutdown();
        let remaining = self.await_drain(timeout).await;
        GracefulOutcome {
            active_at_request,
            drained: active_at_request.saturating_sub(remaining),
            remaining,
        }
    }
}

/// 强杀恢复结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForceKillRecovery {
    /// 陈旧 pid 文件记录的进程号（可能为 None = 内容损坏）。
    pub stale_pid: Option<u32>,
    /// 是否清理了陈旧 pid 文件。
    pub cleaned: bool,
}

/// 进程存活探测（Windows：OpenProcess；其他：kill(pid, 0)）。
pub fn process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        type WinHandle = *mut core::ffi::c_void;
        unsafe extern "system" {
            fn OpenProcess(
                dw_desired_access: u32,
                b_inherit_handle: i32,
                dw_process_id: u32,
            ) -> WinHandle;
            fn GetLastError() -> u32;
            fn CloseHandle(h_object: WinHandle) -> i32;
        }
        const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
        const ERROR_INVALID_PARAMETER: u32 = 87;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            // pid 不存在 → ERROR_INVALID_PARAMETER；权限不足视为存活（保守）。
            return unsafe { GetLastError() } != ERROR_INVALID_PARAMETER;
        }
        unsafe { CloseHandle(handle) };
        true
    }
    #[cfg(not(windows))]
    {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        unsafe { kill(pid as i32, 0) == 0 }
    }
}

/// 服务进程探测：pid 存活 **且** 镜像名是 owo-agent（pid 文件守卫专用）。
///
/// 只看 `process_alive` 不够：强杀后 pid 文件残留、pid 被无关进程（含权限更高的系统进程）复用，
/// 此时 OpenProcess 返回 ACCESS_DENIED 被保守判为「存活」，启动会被误挡（实测停服后立即重启即触发）。
/// Toolhelp 快照读取镜像名不需要目标进程权限，可准确区分。
pub fn is_agent_process(pid: u32) -> bool {
    if !process_alive(pid) {
        return false;
    }
    #[cfg(windows)]
    {
        match process_image_name(pid) {
            Some(name) => {
                name.eq_ignore_ascii_case("owo-agent.exe") || name.eq_ignore_ascii_case("owo-agent")
            }
            // 快照不可用时保守视为服务进程（宁可提示也不要双开写同一数据目录）。
            None => true,
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

/// 通过 Toolhelp 快照读取 pid 对应的镜像名（无需目标进程权限）。
#[cfg(windows)]
fn process_image_name(pid: u32) -> Option<String> {
    use std::os::windows::ffi::OsStringExt;

    type WinHandle = *mut core::ffi::c_void;

    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        dw_size: u32,
        cnt_usage: u32,
        th32_process_id: u32,
        th32_default_heap_id: usize,
        th32_module_id: u32,
        cnt_threads: u32,
        th32_parent_process_id: u32,
        pc_pri_class_base: i32,
        dw_flags: u32,
        sz_exe_file: [u16; 260],
    }

    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> WinHandle;
        fn Process32FirstW(snapshot: WinHandle, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: WinHandle, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(h_object: WinHandle) -> i32;
    }

    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot.is_null() || snapshot as isize == INVALID_HANDLE_VALUE {
        return None;
    }
    let mut entry: ProcessEntry32W = unsafe { std::mem::zeroed() };
    entry.dw_size = std::mem::size_of::<ProcessEntry32W>() as u32;
    let mut name = None;
    let mut has_entry = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while has_entry {
        if entry.th32_process_id == pid {
            let end = entry
                .sz_exe_file
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.sz_exe_file.len());
            name = Some(
                std::ffi::OsString::from_wide(&entry.sz_exe_file[..end])
                    .to_string_lossy()
                    .to_string(),
            );
            break;
        }
        has_entry = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    name
}

/// 服务 pid 文件（存活标记；正常退出 Drop 时清理，强杀后由 recover_force_kill 清理）。
pub struct PidFile {
    path: PathBuf,
    removed: bool,
}

impl PidFile {
    pub fn create(data_root: &std::path::Path) -> Result<Self, String> {
        let path = data_root.join("server.pid");
        std::fs::write(&path, std::process::id().to_string())
            .map_err(|e| format!("写入 pid 文件失败：{e}"))?;
        Ok(Self {
            path,
            removed: false,
        })
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        if !self.removed {
            let _ = std::fs::remove_file(&self.path);
            self.removed = true;
        }
    }
}

/// 强杀恢复：pid 文件存在但进程已死 → 清理（恢复干净状态）；
/// 进程仍存活 → 显式报错（不允许双实例写同一数据目录）。
pub fn recover_force_kill(
    data_root: &std::path::Path,
) -> Result<Option<ForceKillRecovery>, String> {
    let path = data_root.join("server.pid");
    if !path.is_file() {
        return Ok(None);
    }
    let stale_pid = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| content.trim().parse::<u32>().ok());
    match stale_pid {
        Some(pid) if is_agent_process(pid) => {
            return Err(format!(
                "检测到运行中的服务（pid={pid}，pid 文件 {}）：请先停止该进程再启动",
                path.display()
            ));
        }
        _ => {}
    }
    std::fs::remove_file(&path)
        .map_err(|e| format!("清理陈旧 pid 文件失败（{}）：{e}", path.display()))?;
    Ok(Some(ForceKillRecovery {
        stale_pid,
        cleaned: true,
    }))
}
