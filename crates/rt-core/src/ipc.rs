use crossbeam_channel::Sender;
use std::time::Duration;

const IPC_HEADER: &str = "RUSTTRACKER_IPC_V1";
const MAX_IPC_PATHS: usize = 4096;
const MAX_IPC_STREAM_BYTES: u64 = 65_536;
const MAX_IPC_LINE_BYTES: usize = 8192;
const IPC_SOCKET_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line(String),
    TooLong,
}

fn read_bounded_line<R: std::io::BufRead>(
    reader: &mut R,
    raw_buf: &mut Vec<u8>,
) -> std::io::Result<BoundedLine> {
    use std::io::{BufRead, Read};
    raw_buf.clear();
    // Allow +2 bytes so an exact MAX_IPC_LINE_BYTES line terminated by `\r\n` or `\n` fits cleanly
    // without ever slicing inside a multi-byte UTF-8 codepoint.
    let n = reader
        .take((MAX_IPC_LINE_BYTES + 2) as u64)
        .read_until(b'\n', raw_buf)?;
    if n == 0 {
        return Ok(BoundedLine::Eof);
    }
    if !raw_buf.ends_with(b"\n") && raw_buf.len() > MAX_IPC_LINE_BYTES {
        // Drain until newline or outer stream cap so protocol state fails closed cleanly.
        let mut discard = [0u8; 256];
        loop {
            let r = reader.read(&mut discard)?;
            if r == 0 || discard[..r].contains(&b'\n') {
                break;
            }
        }
        return Ok(BoundedLine::TooLong);
    }

    let mut slice = raw_buf.as_slice();
    if let Some(stripped) = slice.strip_suffix(b"\n") {
        slice = stripped;
    }
    if let Some(stripped) = slice.strip_suffix(b"\r") {
        slice = stripped;
    }
    if slice.len() > MAX_IPC_LINE_BYTES {
        return Ok(BoundedLine::TooLong);
    }

    let text = std::str::from_utf8(slice)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(BoundedLine::Line(text.to_string()))
}

fn parse_ipc_request<R: std::io::BufRead>(reader: &mut R) -> Option<Vec<String>> {
    let mut raw_buf = Vec::with_capacity(256);
    let BoundedLine::Line(header) = read_bounded_line(reader, &mut raw_buf).ok()? else {
        return None;
    };
    if header.trim() != IPC_HEADER {
        return None;
    }

    let BoundedLine::Line(count_str) = read_bounded_line(reader, &mut raw_buf).ok()? else {
        return None;
    };
    let raw_count = count_str.trim().parse::<usize>().ok()?;
    if raw_count == 0 || raw_count > MAX_IPC_PATHS {
        return None;
    }

    let mut paths = Vec::with_capacity(raw_count.min(64));
    for _ in 0..raw_count {
        match read_bounded_line(reader, &mut raw_buf) {
            Ok(BoundedLine::Line(line)) => {
                if !line.is_empty() {
                    paths.push(line);
                }
            }
            Ok(BoundedLine::TooLong | BoundedLine::Eof) | Err(_) => {
                return None;
            }
        }
    }

    if paths.is_empty() { None } else { Some(paths) }
}

#[cfg(any(windows, test))]
fn has_complete_ipc_frame(buf: &[u8]) -> bool {
    if buf.len() as u64 >= MAX_IPC_STREAM_BYTES {
        return true;
    }
    let mut lines = buf.split_inclusive(|&b| b == b'\n');
    let Some(hdr) = lines.next() else {
        return false;
    };
    if !hdr.ends_with(b"\n") {
        return hdr.len() > MAX_IPC_LINE_BYTES;
    }
    let hdr_trimmed = hdr.strip_suffix(b"\n").unwrap_or(hdr);
    let hdr_trimmed = hdr_trimmed.strip_suffix(b"\r").unwrap_or(hdr_trimmed);
    if hdr_trimmed.trim_ascii() != IPC_HEADER.as_bytes() {
        return true;
    }

    let Some(cnt_line) = lines.next() else {
        return false;
    };
    if !cnt_line.ends_with(b"\n") {
        return cnt_line.len() > MAX_IPC_LINE_BYTES;
    }
    let cnt_trimmed = cnt_line.strip_suffix(b"\n").unwrap_or(cnt_line);
    let cnt_trimmed = cnt_trimmed.strip_suffix(b"\r").unwrap_or(cnt_trimmed);
    let Ok(cnt_str) = std::str::from_utf8(cnt_trimmed.trim_ascii()) else {
        return true;
    };
    let Ok(expected_paths) = cnt_str.parse::<usize>() else {
        return true;
    };
    if expected_paths == 0 || expected_paths > MAX_IPC_PATHS {
        return true;
    }

    let mut completed_paths = 0usize;
    for line in lines {
        if line.ends_with(b"\n") {
            completed_paths += 1;
            if completed_paths >= expected_paths {
                return true;
            }
        } else if line.len() > MAX_IPC_LINE_BYTES {
            return true;
        }
    }
    false
}

#[cfg(unix)]
mod unix_ipc {
    use super::{
        BoundedLine, IPC_HEADER, IPC_SOCKET_TIMEOUT, MAX_IPC_PATHS, MAX_IPC_STREAM_BYTES,
        parse_ipc_request, read_bounded_line,
    };
    use crossbeam_channel::Sender;
    use std::io::{BufReader, Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};

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
        try_forward_at(&get_socket_path(), paths)
    }

    pub(crate) fn try_forward_at(socket_path: &Path, paths: &[String]) -> bool {
        if paths.is_empty() || !socket_path.exists() {
            return false;
        }

        let stream = match UnixStream::connect(socket_path) {
            Ok(s) => s,
            Err(_) => {
                // Stale socket from a previous crashed run -> cleanup
                let _ = std::fs::remove_file(socket_path);
                return false;
            }
        };

        let _ = stream.set_read_timeout(Some(IPC_SOCKET_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IPC_SOCKET_TIMEOUT));

        let mut writer = match stream.try_clone() {
            Ok(w) => w,
            Err(_) => return false,
        };
        let mut reader = BufReader::new(stream.take(MAX_IPC_STREAM_BYTES));

        let send_paths = &paths[..paths.len().min(MAX_IPC_PATHS)];
        let mut payload = format!("{IPC_HEADER}\n{}\n", send_paths.len());
        for p in send_paths {
            payload.push_str(p);
            payload.push('\n');
        }

        if writer.write_all(payload.as_bytes()).is_err() || writer.flush().is_err() {
            return false;
        }

        let mut raw_buf = Vec::with_capacity(16);
        matches!(
            read_bounded_line(&mut reader, &mut raw_buf),
            Ok(BoundedLine::Line(resp)) if resp.trim() == "OK"
        )
    }

    pub fn start_server(tx: Sender<Vec<String>>) {
        start_server_at(get_socket_path(), tx);
    }

    pub(crate) fn start_server_at(socket_path: PathBuf, tx: Sender<Vec<String>>) {
        let _ = std::fs::remove_file(&socket_path);

        let listener = match UnixListener::bind(&socket_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "[ipc] Failed to bind UNIX domain socket {:?}: {}",
                    socket_path, e
                );
                return;
            }
        };

        std::thread::Builder::new()
            .name("rusttracker-ipc".into())
            .spawn(move || {
                for mut stream in listener.incoming().flatten() {
                    let _ = stream.set_read_timeout(Some(IPC_SOCKET_TIMEOUT));
                    let _ = stream.set_write_timeout(Some(IPC_SOCKET_TIMEOUT));
                    let cloned = match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut reader = BufReader::new(cloned.take(MAX_IPC_STREAM_BYTES));
                    if let Some(paths) = parse_ipc_request(&mut reader) {
                        let _ = tx.send(paths);
                        let _ = stream.write_all(b"OK\n");
                        let _ = stream.flush();
                    }
                }
            })
            .expect("Failed to spawn IPC listener thread");
    }
}

#[cfg(windows)]
mod windows_ipc {
    use super::{
        BoundedLine, IPC_HEADER, IPC_SOCKET_TIMEOUT, MAX_IPC_LINE_BYTES, MAX_IPC_PATHS,
        MAX_IPC_STREAM_BYTES, has_complete_ipc_frame, parse_ipc_request, read_bounded_line,
    };
    use crossbeam_channel::Sender;
    use std::io::{Cursor, Read, Write};
    use std::os::windows::io::FromRawHandle;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::{
        CloseHandle, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_MODE, FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT, PeekNamedPipe,
    };
    use windows::core::w;

    const PIPE_NAME: windows::core::PCWSTR = w!(r"\\.\pipe\RustTracker-SingleInstance");

    /// Reads available bytes from a synchronous Win32 named pipe without blocking indefinitely
    /// by polling `PeekNamedPipe` up to `deadline` and reading only already-buffered bytes.
    fn read_pipe_bounded(
        pipe_file: &mut std::fs::File,
        raw_handle: HANDLE,
        deadline: Instant,
        stop_pred: impl Fn(&[u8]) -> bool,
    ) -> std::io::Result<Vec<u8>> {
        let mut out = Vec::with_capacity(256);
        let mut chunk = [0u8; 4096];
        loop {
            let mut avail: u32 = 0;
            if unsafe { PeekNamedPipe(raw_handle, None, 0, None, Some(&mut avail), None) }.is_err()
            {
                break;
            }
            if avail > 0 {
                let remaining_cap = (MAX_IPC_STREAM_BYTES as usize).saturating_sub(out.len());
                if remaining_cap == 0 {
                    break;
                }
                let to_read = (avail as usize).min(chunk.len()).min(remaining_cap);
                let n = pipe_file.read(&mut chunk[..to_read])?;
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&chunk[..n]);
                if stop_pred(&out) {
                    break;
                }
            } else {
                if stop_pred(&out) || Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(out)
    }

    pub fn try_forward(paths: &[String]) -> bool {
        if paths.is_empty() {
            return false;
        }
        let Ok(handle) = (unsafe {
            CreateFileW(
                PIPE_NAME,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                FILE_SHARE_MODE(0),
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }) else {
            return false;
        };
        if handle == INVALID_HANDLE_VALUE {
            return false;
        }

        let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };

        let send_paths = &paths[..paths.len().min(MAX_IPC_PATHS)];
        let mut payload = format!("{IPC_HEADER}\n{}\n", send_paths.len());
        for p in send_paths {
            payload.push_str(p);
            payload.push('\n');
        }

        if file.write_all(payload.as_bytes()).is_err() {
            return false;
        }
        let _ = unsafe { FlushFileBuffers(handle) };

        let Ok(resp_bytes) = read_pipe_bounded(
            &mut file,
            handle,
            Instant::now() + IPC_SOCKET_TIMEOUT,
            |b| b.contains(&b'\n') || b.len() > MAX_IPC_LINE_BYTES,
        ) else {
            return false;
        };

        let mut cursor = Cursor::new(resp_bytes);
        let mut raw_buf = Vec::with_capacity(16);
        matches!(
            read_bounded_line(&mut cursor, &mut raw_buf),
            Ok(BoundedLine::Line(resp)) if resp.trim() == "OK"
        )
    }

    pub fn start_server(tx: Sender<Vec<String>>) {
        std::thread::Builder::new()
            .name("rusttracker-ipc-win".into())
            .spawn(move || {
                loop {
                    let pipe_handle = unsafe {
                        CreateNamedPipeW(
                            PIPE_NAME,
                            PIPE_ACCESS_DUPLEX,
                            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                            PIPE_UNLIMITED_INSTANCES,
                            8192,
                            8192,
                            500,
                            None,
                        )
                    };

                    if pipe_handle == INVALID_HANDLE_VALUE {
                        eprintln!("[ipc] Failed to create named pipe");
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }

                    let connected = match unsafe { ConnectNamedPipe(pipe_handle, None) } {
                        Ok(()) => true,
                        Err(e) => e.code() == ERROR_PIPE_CONNECTED.to_hresult(),
                    };

                    if connected {
                        let mut file = unsafe { std::fs::File::from_raw_handle(pipe_handle.0) };
                        if let Ok(req_bytes) = read_pipe_bounded(
                            &mut file,
                            pipe_handle,
                            Instant::now() + IPC_SOCKET_TIMEOUT,
                            has_complete_ipc_frame,
                        ) {
                            let mut cursor = Cursor::new(req_bytes);
                            if let Some(paths) = parse_ipc_request(&mut cursor) {
                                let _ = tx.send(paths);
                                if file.write_all(b"OK\n").is_ok() {
                                    let _ = unsafe { FlushFileBuffers(pipe_handle) };
                                }
                            }
                        }
                        let _ = unsafe { DisconnectNamedPipe(pipe_handle) };
                        drop(file);
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
    fn test_read_bounded_line_multibyte_utf8_at_boundary_never_panics() {
        // 8191 ASCII bytes + 2-byte UTF-8 'é' (0xC3 0xA9 spanning indices 8191..8193) + "x\n"
        // Previously panicked inside `String::truncate(8192)` on a non-char boundary.
        let mut payload = Vec::new();
        payload.extend_from_slice(b"RUSTTRACKER_IPC_V1\n1\n/");
        payload.extend(std::iter::repeat_n(b'a', MAX_IPC_LINE_BYTES - 2));
        payload.extend_from_slice("éx\n".as_bytes());

        let mut cursor = std::io::Cursor::new(payload);
        assert!(
            parse_ipc_request(&mut cursor).is_none(),
            "Overlong multi-byte UTF-8 line crossing MAX_IPC_LINE_BYTES must fail closed without panicking"
        );

        // Exact-limit UTF-8 line (8192 bytes including multi-byte char) with CRLF must succeed
        let mut exact_line = "/".to_string();
        exact_line.push_str(&"a".repeat(MAX_IPC_LINE_BYTES - 3));
        exact_line.push('é'); // 2 UTF-8 bytes -> total 8192 bytes
        assert_eq!(exact_line.len(), MAX_IPC_LINE_BYTES);
        let valid_frame = format!("RUSTTRACKER_IPC_V1\r\n1\r\n{exact_line}\r\n");
        let mut valid_cursor = std::io::Cursor::new(valid_frame.into_bytes());
        let parsed =
            parse_ipc_request(&mut valid_cursor).expect("Exact 8192-byte UTF-8 path must parse");
        assert_eq!(parsed, vec![exact_line]);
    }

    #[test]
    fn test_has_complete_ipc_frame_detection() {
        assert!(!has_complete_ipc_frame(b""));
        assert!(!has_complete_ipc_frame(b"RUSTTRACKER_IPC_V1\n2\n/a.mp3\n"));
        assert!(has_complete_ipc_frame(
            b"RUSTTRACKER_IPC_V1\n2\n/a.mp3\n/b.flac\n"
        ));
        // Invalid header and huge count fail fast without waiting for deadline
        assert!(has_complete_ipc_frame(b"BOGUS_HEADER\n"));
        assert!(has_complete_ipc_frame(b"RUSTTRACKER_IPC_V1\n99999999\n"));
    }

    #[test]
    #[cfg(unix)]
    fn test_ipc_communication_unix() {
        use std::io::Write;
        use std::os::unix::net::UnixStream;

        let unique_sock = std::env::temp_dir().join(format!(
            "rusttracker-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));

        let (tx, rx) = crossbeam_channel::unbounded();
        unix_ipc::start_server_at(unique_sock.clone(), tx);
        for _ in 0..20 {
            if unique_sock.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // 1. Adversarial huge count (e.g. usize::MAX) must be rejected immediately without OOM
        {
            let mut s = UnixStream::connect(&unique_sock).expect("connect adversarial count");
            let _ = s.write_all(format!("RUSTTRACKER_IPC_V1\n{}\n/a.mp3\n", usize::MAX).as_bytes());
            let _ = s.flush();
        }
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "Oversized IPC count must be rejected without forwarding"
        );

        // 2. Oversized multi-byte UTF-8 single-line payload must be rejected (never panicking or forwarding a mangled half-path)
        {
            let mut s = UnixStream::connect(&unique_sock).expect("connect oversized utf8 line");
            let mut msg = b"RUSTTRACKER_IPC_V1\n1\n/".to_vec();
            msg.extend(std::iter::repeat_n(b'a', MAX_IPC_LINE_BYTES - 2));
            msg.extend_from_slice("éx\n".as_bytes());
            let _ = s.write_all(&msg);
            let _ = s.flush();
        }
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(50))
                .is_err(),
            "Overlong path line must be rejected rather than truncated into a bogus path"
        );

        // 3. Valid roundtrip on the isolated test socket proves the server survived the adversarial frames
        let files = vec![
            "/tmp/sample1.mp3".to_string(),
            "/tmp/sample2.flac".to_string(),
        ];
        let forwarded = unix_ipc::try_forward_at(&unique_sock, &files);
        assert!(
            forwarded,
            "try_forward_at should succeed when server is running"
        );

        let received = rx
            .recv_timeout(std::time::Duration::from_millis(1000))
            .expect("Should receive forwarded paths");
        assert_eq!(received, files);

        // Clean up isolated test socket
        let _ = std::fs::remove_file(&unique_sock);

        // Verify forwarding returns false when socket does not exist
        let files2 = vec!["/tmp/sample3.mp3".to_string()];
        let forwarded2 = unix_ipc::try_forward_at(&unique_sock, &files2);
        assert!(
            !forwarded2,
            "try_forward_at should return false when no socket exists"
        );
    }
}
