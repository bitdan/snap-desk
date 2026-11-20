//! 单实例：同一用户会话只运行一个 GPUI 版进程，再次启动时把命令行参数转交给首个实例。
//!
//! - Windows：`Local\` 命名互斥量判断是否首个实例；命名管道接收后续实例的参数。
//!   互斥量随进程退出由系统释放，不会因崩溃残留。
//! - 其他平台：Unix 域套接字既是锁也是通道；残留的套接字文件（上次崩溃）连接失败后删除重建。
//!
//! 标识与 Tauri 版（tauri-plugin-single-instance）不同，迁移期间两个程序可以同时运行；
//! 但它们读取同一份快捷键设置，后启动者注册快捷键会失败并记录日志。
//! 协议：客户端写入以 `\0` 分隔的参数（不含程序路径），写完关闭连接。

/// 启动结果：首个实例拿到接收器；后续实例已转交参数，应直接退出。
pub enum Instance {
    Primary(Listener),
    Secondary,
}

/// 首个实例持有的锁与监听端；`start` 后在后台线程逐个接收转交的参数。
pub struct Listener {
    inner: platform::Listener,
}

impl Listener {
    /// 每次收到参数在后台线程调用 `on_args`；回调应只把参数送进通道，不直接操作界面。
    pub fn start(self, on_args: impl Fn(Vec<String>) + Send + 'static) {
        self.inner.start(on_args);
    }
}

pub fn acquire(args: &[String]) -> Instance {
    match platform::acquire(args) {
        Ok(Some(inner)) => Instance::Primary(Listener { inner }),
        Ok(None) => Instance::Secondary,
        Err(error) => {
            // 锁机制本身不可用时宁可多开，也不让用户无法启动。
            log::warn!("event=gpui.single_instance_unavailable error={error}");
            Instance::Primary(Listener {
                inner: platform::Listener::disabled(),
            })
        }
    }
}

fn decode(bytes: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(bytes)
        .split('\0')
        .filter(|arg| !arg.is_empty())
        .map(str::to_owned)
        .collect()
}

fn encode(args: &[String]) -> Vec<u8> {
    args.join("\0").into_bytes()
}

/// 名称里带用户名：命名管道全局可见，避免同机多用户互相转交参数。
fn user_tag() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect()
}

#[cfg(windows)]
mod platform {
    use std::{io::Write, time::Duration};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, HANDLE,
            INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND},
        System::{
            Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
            },
            Threading::CreateMutexW,
        },
    };

    const NAME: &str = "io.snapdesk.app.gpui";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn pipe_name() -> String {
        format!(r"\\.\pipe\{NAME}-{}", super::user_tag())
    }

    /// 句柄只在本进程内使用；包一层让它能移入后台线程（HANDLE 是裸指针，默认不是 Send）。
    struct Handle(HANDLE);
    // SAFETY: 管道句柄只由接收线程独占使用，互斥量句柄只在进程退出时由系统关闭。
    unsafe impl Send for Handle {}

    pub struct Listener {
        /// 持有互斥量直到进程退出；None 表示锁不可用（降级为允许多开）。
        _mutex: Option<Handle>,
        pipe: Option<Handle>,
    }

    impl Listener {
        pub fn disabled() -> Self {
            Self { _mutex: None, pipe: None }
        }

        pub fn start(self, on_args: impl Fn(Vec<String>) + Send + 'static) {
            // Handle 没有 Drop：互斥量句柄不会随 Listener 析构关闭，锁一直保持到进程退出。
            let Some(pipe) = self.pipe else { return };
            let spawned = std::thread::Builder::new()
                .name("single-instance".into())
                .spawn(move || serve(pipe, on_args));
            if spawned.is_err() {
                log::warn!("event=gpui.single_instance_thread_failed");
            }
        }
    }

    /// 单个管道实例反复 Connect/Disconnect：始终占着管道名，没有“换实例”的空档。
    fn serve(pipe: Handle, on_args: impl Fn(Vec<String>)) {
        loop {
            // SAFETY: pipe.0 是本函数独占的有效管道句柄；同步模式下 overlapped 传空。
            let connected = unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) } != 0
                || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
            if connected {
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let mut read = 0u32;
                    // SAFETY: buffer 在调用期间有效，长度与传入一致。
                    let ok = unsafe {
                        ReadFile(
                            pipe.0,
                            buffer.as_mut_ptr(),
                            buffer.len() as u32,
                            &mut read,
                            std::ptr::null_mut(),
                        )
                    } != 0;
                    if !ok || read == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..read as usize]);
                    // 参数很短；超过上限视为异常客户端，丢弃剩余内容。
                    if bytes.len() > 64 * 1024 {
                        break;
                    }
                }
                on_args(super::decode(&bytes));
            }
            // SAFETY: 断开当前客户端后同一句柄可再次等待连接。
            unsafe { DisconnectNamedPipe(pipe.0) };
        }
    }

    pub fn acquire(args: &[String]) -> Result<Option<Listener>, String> {
        let mutex_name = wide(&format!(r"Local\{NAME}"));
        // SAFETY: 名称以 0 结尾；默认安全属性；返回句柄由 Listener 持有。
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        if mutex.is_null() {
            return Err(format!("CreateMutexW 失败：{}", unsafe { GetLastError() }));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // SAFETY: 后续实例不需要互斥量。
            unsafe { CloseHandle(mutex) };
            forward(args);
            return Ok(None);
        }
        let name = wide(&pipe_name());
        // SAFETY: FIRST_PIPE_INSTANCE 保证管道名不被其他进程抢先占用后由我们“共享”。
        let pipe = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        let pipe = if pipe == INVALID_HANDLE_VALUE {
            // 仍是首个实例，只是无法接收转交参数（例如管道名被占用）。
            log::warn!("event=gpui.single_instance_pipe_failed code={}", unsafe { GetLastError() });
            None
        } else {
            Some(Handle(pipe))
        };
        Ok(Some(Listener {
            _mutex: Some(Handle(mutex)),
            pipe,
        }))
    }

    /// 首个实例可能正在处理上一个连接（管道忙）；短暂重试，仍失败只记录日志。
    fn forward(args: &[String]) {
        let name = pipe_name();
        for _ in 0..20 {
            match std::fs::OpenOptions::new().write(true).open(&name) {
                Ok(mut pipe) => {
                    let _ = pipe.write_all(&super::encode(args));
                    return;
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        log::warn!("event=gpui.single_instance_forward_failed");
    }
}

#[cfg(unix)]
mod platform {
    use std::{
        io::{Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::PathBuf,
    };

    fn socket_path() -> PathBuf {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        dir.join(format!("io.snapdesk.app.gpui-{}.sock", super::user_tag()))
    }

    pub struct Listener {
        socket: Option<UnixListener>,
    }

    impl Listener {
        pub fn disabled() -> Self {
            Self { socket: None }
        }

        pub fn start(self, on_args: impl Fn(Vec<String>) + Send + 'static) {
            let Some(socket) = self.socket else { return };
            let spawned = std::thread::Builder::new()
                .name("single-instance".into())
                .spawn(move || {
                    for stream in socket.incoming().flatten() {
                        let mut bytes = Vec::new();
                        let _ = stream.take(64 * 1024).read_to_end(&mut bytes);
                        on_args(super::decode(&bytes));
                    }
                });
            if spawned.is_err() {
                log::warn!("event=gpui.single_instance_thread_failed");
            }
        }
    }

    pub fn acquire(args: &[String]) -> Result<Option<Listener>, String> {
        let path = socket_path();
        match UnixListener::bind(&path) {
            Ok(socket) => return Ok(Some(Listener { socket: Some(socket) })),
            Err(error) if error.kind() != std::io::ErrorKind::AddrInUse => {
                return Err(error.to_string());
            }
            Err(_) => {}
        }
        if let Ok(mut stream) = UnixStream::connect(&path) {
            let _ = stream.write_all(&super::encode(args));
            return Ok(None);
        }
        // 套接字文件残留但无人监听：上次异常退出，删除后重新占用。
        let _ = std::fs::remove_file(&path);
        UnixListener::bind(&path)
            .map(|socket| Some(Listener { socket: Some(socket) }))
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_round_trip_without_empty_entries() {
        let args = vec!["--action".to_string(), "capture".to_string()];
        assert_eq!(decode(&encode(&args)), args);
        assert!(decode(b"").is_empty());
    }
}
