//! 专项基准的进程内存读数，仅编入测试；不采集屏幕、命令行或用户文件。
/// Windows 私有提交量与工作集分开记录；峰值属于整个测试进程生命周期，不能当作单阶段增量。
pub(crate) fn memory() -> serde_json::Value {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            faults: u32,
            peak_working_set: usize,
            working_set: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            commit: usize,
            peak_commit: usize,
            private: usize,
        }
        #[link(name = "psapi")]
        extern "system" {
            fn GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                counters: *mut Counters,
                size: u32,
            ) -> i32;
        }
        let mut counters = Counters::default();
        counters.cb = std::mem::size_of::<Counters>() as u32;
        let success = unsafe {
            GetProcessMemoryInfo(
                windows::Win32::System::Threading::GetCurrentProcess().0,
                &mut counters,
                std::mem::size_of::<Counters>() as u32,
            )
        };
        if success != 0 {
            return serde_json::json!({"private_bytes": counters.private, "working_set_bytes": counters.working_set,
                "peak_working_set_bytes": counters.peak_working_set, "peak_commit_bytes": counters.peak_commit, "peak_scope":"process_lifetime"});
        }
    }
    serde_json::json!({"private_bytes":null, "working_set_bytes":null, "peak_working_set_bytes":null, "peak_commit_bytes":null})
}
