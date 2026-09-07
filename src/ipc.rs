use crossbeam_channel::Sender;

#[cfg(unix)]
mod unix_ipc {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;
    use crossbeam_channel::Sender;

    pub(crate) fn get_socket_path() -> PathBuf {
        if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
            let p = PathBuf::from(runtime_dir);
            if p.is_dir() {
                return p.join("rusttracker.sock");
            }
        }
        let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        std::env::temp_dir().join(format!("rusttracker-{}.sock", user))
    }

    pub fn try_forward(paths: &[String]) -> bool {
        let socket_path = get_socket_path();
        if !socket_path.exists() {
            return false;
        }

        let stream = match UnixStream::connect(&socket_path) {
            Ok(s) => s,
            Err(_) => {
                // Stale socket from a previous crashed run -> cleanup
                let _ = std::fs::remove_file(&socket_path);
                return false;
            }
        };

        let _ = stream.set_read_timeout(Some(Duration::from_millis(1500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(1500)));

        let mut writer = match stream.try_clone() {
            Ok(w) => w,
            Err(_) => return false,
        };
        let mut reader = BufReader::new(stream);

        let mut payload = format!("RUSTTRACKER_IPC_V1\n{}\n", paths.len());
        for p in paths {
            payload.push_str(p);
            payload.push('\n');
        }

        if writer.write_all(payload.as_bytes()).is_err() || writer.flush().is_err() {
            return false;
        }

        let mut response = String::new();
        if reader.read_line(&mut response).is_ok() && response.trim() == "OK" {
            return true;
        }

        false
    }

    pub fn start_server(tx: Sender<Vec<String>>) {
        let socket_path = get_socket_path();
        let _ = std::fs::remove_file(&socket_path);

        let listener = match UnixListener::bind(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[ipc] Failed to bind UNIX domain socket {:?}: {}", socket_path, e);
                return;
            }
        };

        std::thread::Builder::new()
            .name("rusttracker-ipc".into())
            .spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let cloned = match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut reader = BufReader::new(cloned);
                    let mut header = String::new();
                    if reader.read_line(&mut header).is_err() || header.trim() != "RUSTTRACKER_IPC_V1" {
                        continue;
                    }

                    let mut count_str = String::new();
                    if reader.read_line(&mut count_str).is_err() {
                        continue;
                    }
                    let count: usize = count_str.trim().parse().unwrap_or(0);
                    let mut paths = Vec::with_capacity(count);
                    for _ in 0..count {
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_ok() {
                            let trimmed = line.trim_end_matches(['\r', '\n']);
                            if !trimmed.is_empty() {
                                paths.push(trimmed.to_string());
                            }
                        }
                    }

                    let _ = tx.send(paths);
                    let _ = stream.write_all(b"OK\n");
                    let _ = stream.flush();
                }
            })
            .expect("Failed to spawn IPC listener thread");
    }
}

#[cfg(windows)]
mod windows_ipc {
    use std::io::{BufRead, BufReader, Write};
    use crossbeam_channel::Sender;
    use windows::core::w;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
        PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    const PIPE_NAME: windows::core::PCWSTR = w!(r"\\.\pipe\RustTracker-SingleInstance");

    pub fn try_forward(paths: &[String]) -> bool {
        // Attempt to connect to the existing named pipe
        let handle = unsafe {
            CreateFileW(
                PIPE_NAME,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                windows::Win32::Storage::FileSystem::FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };

        if handle.is_err() || handle == Ok(INVALID_HANDLE_VALUE) {
            return false;
        }

        let handle = handle.unwrap();
        use std::os::windows::io::FromRawHandle;
        let file = unsafe { std::fs::File::from_raw_handle(handle.0) };

        let mut writer = match file.try_clone() {
            Ok(w) => w,
            Err(_) => return false,
        };
        let mut reader = BufReader::new(file);

        let mut payload = format!("RUSTTRACKER_IPC_V1\n{}\n", paths.len());
        for p in paths {
            payload.push_str(p);
            payload.push('\n');
        }

        if writer.write_all(payload.as_bytes()).is_err() || writer.flush().is_err() {
            return false;
        }

        let mut response = String::new();
        if reader.read_line(&mut response).is_ok() && response.trim() == "OK" {
            return true;
        }

        false
    }

    pub fn start_server(tx: Sender<Vec<String>>) {
        std::thread::Builder::new()
            .name("rusttracker-ipc-win".into())
            .spawn(move || {
                use std::os::windows::io::FromRawHandle;
                loop {
                    let pipe_handle = unsafe {
                        CreateNamedPipeW(
                            PIPE_NAME,
                            PIPE_ACCESS_DUPLEX,
                            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                            PIPE_UNLIMITED_INSTANCES,
                            8192,
                            8192,
                            0,
                            None,
                        )
                    };

                    if pipe_handle == INVALID_HANDLE_VALUE {
                        eprintln!("[ipc] Failed to create named pipe");
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        continue;
                    }

                    let connected = unsafe { ConnectNamedPipe(pipe_handle, None) }.is_ok();
                    if connected {
                        let file = unsafe { std::fs::File::from_raw_handle(pipe_handle.0) };
                        let mut reader = BufReader::new(match file.try_clone() {
                            Ok(f) => f,
                            Err(_) => {
                                let _ = unsafe { DisconnectNamedPipe(pipe_handle) };
                                continue;
                            }
                        });
                        let mut writer = file;

                        let mut header = String::new();
                        if reader.read_line(&mut header).is_ok() && header.trim() == "RUSTTRACKER_IPC_V1" {
                            let mut count_str = String::new();
                            if reader.read_line(&mut count_str).is_ok() {
                                let count: usize = count_str.trim().parse().unwrap_or(0);
                                let mut paths = Vec::with_capacity(count);
                                for _ in 0..count {
                                    let mut line = String::new();
                                    if reader.read_line(&mut line).is_ok() {
                                        let trimmed = line.trim_end_matches(['\r', '\n']);
                                        if !trimmed.is_empty() {
                                            paths.push(trimmed.to_string());
                                        }
                                    }
                                }

                                let _ = tx.send(paths);
                                let _ = writer.write_all(b"OK\n");
                                let _ = writer.flush();
                            }
                        }

                        let _ = unsafe { DisconnectNamedPipe(pipe_handle) };
                    } else {
                        let _ = unsafe { CloseHandle(pipe_handle) };
                    }
                }
            })
            .expect("Failed to spawn Windows IPC thread");
    }
}

/// Attempts to forward input paths to an already running instance of RustTracker.
/// Returns true if an existing instance was running and received the payload.
pub fn try_forward_to_existing_instance(paths: &[String]) -> bool {
    #[cfg(unix)]
    {
        unix_ipc::try_forward(paths)
    }
    #[cfg(windows)]
    {
        windows_ipc::try_forward(paths)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = paths;
        false
    }
}

/// Starts the IPC server to listen for new files launched by secondary instances.
pub fn start_ipc_server(tx: Sender<Vec<String>>) {
    #[cfg(unix)]
    {
        unix_ipc::start_server(tx);
    }
    #[cfg(windows)]
    {
        windows_ipc::start_server(tx);
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = tx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_ipc_communication_unix() {
        let (tx, rx) = crossbeam_channel::unbounded();
        start_ipc_server(tx);
        std::thread::sleep(std::time::Duration::from_millis(100));

        let files = vec!["/tmp/sample1.mp3".to_string(), "/tmp/sample2.flac".to_string()];
        let forwarded = try_forward_to_existing_instance(&files);
        assert!(forwarded, "try_forward_to_existing_instance should succeed when server is running");

        let received = rx.recv_timeout(std::time::Duration::from_millis(1000)).expect("Should receive forwarded paths");
        assert_eq!(received, files);

        // Clean up socket
        let socket_path = unix_ipc::get_socket_path();
        let _ = std::fs::remove_file(socket_path);

        // Verify forwarding returns false when socket does not exist
        let files2 = vec!["/tmp/sample3.mp3".to_string()];
        let forwarded2 = try_forward_to_existing_instance(&files2);
        assert!(!forwarded2, "try_forward_to_existing_instance should return false when no socket exists");
    }
}
