//! Bounded, unauthenticated text retrieval with an optional caller-owned cache.
//! Remote content is data; callers must not treat it as instructions.

use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

fn default_timeout() -> u64 {
    15_000
}
fn default_max_bytes() -> usize {
    1024 * 1024
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchRequest {
    pub url: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_bytes")]
    pub max_bytes: usize,
    #[serde(default)]
    pub cache_ttl_ms: Option<u64>,
    #[serde(default)]
    pub allow_http: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchResponse {
    pub url: String,
    pub status: u32,
    pub content_type: Option<String>,
    pub body: String,
    pub fetched_at_ms: u64,
    pub cached: bool,
}

pub fn fetch(request: &FetchRequest, store: Option<&Store>) -> crate::Result<FetchResponse> {
    fetch_with(request, store, native::fetch)
}

fn fetch_with(
    request: &FetchRequest,
    store: Option<&Store>,
    transport: impl FnOnce(&FetchRequest) -> crate::Result<FetchResponse>,
) -> crate::Result<FetchResponse> {
    if request.url.is_empty() || request.url.len() > 8192 || request.url.contains(['\0', '#']) {
        return Err("URL must be 1 to 8192 bytes and contain no fragment or NUL".into());
    }
    if !(1..=60_000).contains(&request.timeout_ms) {
        return Err("timeout_ms must be between 1 and 60000".into());
    }
    if !(1..=8 * 1024 * 1024).contains(&request.max_bytes) {
        return Err("max_bytes must be between 1 and 8388608".into());
    }
    native::validate_url(request)?;
    let ttl = request.cache_ttl_ms.unwrap_or(0);
    let now = now_ms()?;
    if ttl > 0 {
        if let Some(db) = store {
            if let Some(value) = db.cache_get("rhk.http", &request.url)? {
                let mut cached: FetchResponse =
                    serde_json::from_value(value).map_err(|e| e.to_string())?;
                if cached.url == request.url
                    && cached.status == 200
                    && cached.body.len() <= request.max_bytes
                    && now >= cached.fetched_at_ms
                    && now - cached.fetched_at_ms < ttl
                {
                    cached.cached = true;
                    return Ok(cached);
                }
            }
        }
    }
    let result = transport(request)?;
    if result.body.len() > request.max_bytes {
        return Err("response exceeds max_bytes".into());
    }
    if result.status == 200 && ttl > 0 {
        if let Some(db) = store {
            db.cache_put(
                "rhk.http",
                &request.url,
                serde_json::to_value(&result).map_err(|e| e.to_string())?,
                ttl,
            )?;
        }
    }
    Ok(result)
}

fn now_ms() -> crate::Result<u64> {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis(),
    )
    .map_err(|e| e.to_string())
}

#[cfg(not(windows))]
mod native {
    use super::*;
    pub fn validate_url(_: &FetchRequest) -> crate::Result<()> {
        Err("web fetch requires Windows".into())
    }
    pub fn fetch(_: &FetchRequest) -> crate::Result<FetchResponse> {
        Err("web fetch requires Windows".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    fn server(response: &'static str) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).unwrap();
            stream.write_all(response.as_bytes()).unwrap();
        });
        (url, worker)
    }
    fn request(url: String) -> FetchRequest {
        FetchRequest {
            url,
            timeout_ms: 2000,
            max_bytes: 100,
            cache_ttl_ms: None,
            allow_http: true,
        }
    }
    #[test]
    fn native_limits_and_redirects() {
        let (url, worker) =
            server("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest");
        let mut req = request(url);
        req.max_bytes = 3;
        assert!(fetch(&req, None).unwrap_err().contains("max_bytes"));
        worker.join().unwrap();
        let (url, worker) = server(
            "HTTP/1.1 302 Found\r\nLocation: /other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        assert!(
            fetch(&request(url), None)
                .unwrap_err()
                .contains("redirect 302")
        );
        worker.join().unwrap();
        let mut req = request("http://user:pass@127.0.0.1/".into());
        assert!(fetch(&req, None).is_err());
        req.url = "http://127.0.0.1/".into();
        req.allow_http = false;
        assert!(fetch(&req, None).is_err());
    }
    #[test]
    fn cache_hits_expire_and_respect_limits() {
        let path = std::env::temp_dir().join(format!("rhk-web-test-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = Store::open(&path).unwrap();
        let (url, worker) =
            server("HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest");
        let mut req = request(url);
        req.cache_ttl_ms = Some(1000);
        let first = fetch(&req, Some(&db)).unwrap();
        worker.join().unwrap();
        assert!(!first.cached);
        assert_eq!(first.body, "test");
        assert!(fetch(&req, Some(&db)).unwrap().cached);
        req.max_bytes = 3;
        assert!(fetch(&req, Some(&db)).is_err());
        req.max_bytes = 100;
        req.cache_ttl_ms = Some(1);
        thread::sleep(Duration::from_millis(5));
        assert!(fetch(&req, Some(&db)).is_err());
        drop(db);
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{ffi::c_void, ptr};
    use windows::{
        Win32::Networking::WinHttp::*,
        core::{PCWSTR, w},
    };

    struct Handle(*mut c_void);
    impl Handle {
        fn new(handle: *mut c_void) -> crate::Result<Self> {
            if handle.is_null() {
                Err(std::io::Error::last_os_error().to_string())
            } else {
                Ok(Self(handle))
            }
        }
    }
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }

    struct Parts {
        host: Vec<u16>,
        path: Vec<u16>,
        port: u16,
        secure: bool,
    }
    fn parse(request: &FetchRequest) -> crate::Result<Parts> {
        let url: Vec<u16> = request.url.encode_utf16().collect();
        let mut c = URL_COMPONENTS {
            dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
            dwSchemeLength: 1,
            dwHostNameLength: 1,
            dwUserNameLength: 1,
            dwPasswordLength: 1,
            dwUrlPathLength: 1,
            dwExtraInfoLength: 1,
            ..Default::default()
        };
        unsafe {
            WinHttpCrackUrl(&url, 0, &mut c).map_err(|e| format!("invalid URL: {e}"))?;
        }
        let secure = c.nScheme == WINHTTP_INTERNET_SCHEME_HTTPS;
        if !secure && (c.nScheme != WINHTTP_INTERNET_SCHEME_HTTP || !request.allow_http) {
            return Err("HTTPS is required unless HTTP is explicitly enabled".into());
        }
        if c.dwHostNameLength == 0 || c.dwUserNameLength != 0 || c.dwPasswordLength != 0 {
            return Err("URL must have a host and no embedded credentials".into());
        }
        // Reject even empty userinfo, which WinHttpCrackUrl may report with zero length.
        let authority = request
            .url
            .split_once("://")
            .map(|(_, rest)| rest.split(['/', '?']).next().unwrap_or(""))
            .unwrap_or("");
        if authority.contains('@') {
            return Err("URL must not contain embedded credentials".into());
        }
        unsafe fn part(p: windows::core::PWSTR, len: u32) -> Vec<u16> {
            if len == 0 {
                Vec::new()
            } else {
                unsafe { std::slice::from_raw_parts(p.0, len as usize).to_vec() }
            }
        }
        let mut host = unsafe { part(c.lpszHostName, c.dwHostNameLength) };
        host.push(0);
        let mut path = unsafe { part(c.lpszUrlPath, c.dwUrlPathLength) };
        if path.is_empty() {
            path.push(b'/' as u16);
        }
        path.extend(unsafe { part(c.lpszExtraInfo, c.dwExtraInfoLength) });
        path.push(0);
        Ok(Parts {
            host,
            path,
            port: c.nPort,
            secure,
        })
    }
    pub fn validate_url(request: &FetchRequest) -> crate::Result<()> {
        parse(request).map(|_| ())
    }

    fn header(handle: &Handle, kind: u32) -> Option<String> {
        let mut len = 0u32;
        let mut index = 0u32;
        unsafe {
            let _ = WinHttpQueryHeaders(handle.0, kind, PCWSTR::null(), None, &mut len, &mut index);
        }
        if len == 0 || len > 65536 {
            return None;
        }
        let mut buf = vec![0u16; (len as usize + 1) / 2];
        unsafe {
            WinHttpQueryHeaders(
                handle.0,
                kind,
                PCWSTR::null(),
                Some(buf.as_mut_ptr().cast()),
                &mut len,
                &mut index,
            )
            .ok()?;
        }
        buf.truncate(len as usize / 2);
        while buf.last() == Some(&0) {
            buf.pop();
        }
        String::from_utf16(&buf).ok()
    }
    pub fn fetch(request: &FetchRequest) -> crate::Result<FetchResponse> {
        let p = parse(request)?;
        unsafe {
            let session = Handle::new(WinHttpOpen(
                w!("RustyHotkey/1"),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ))?;
            let t = request.timeout_ms as i32;
            WinHttpSetTimeouts(session.0, t, t, t, t).map_err(|e| e.to_string())?;
            let connection = Handle::new(WinHttpConnect(
                session.0,
                PCWSTR(p.host.as_ptr()),
                p.port,
                0,
            ))?;
            let flags = if p.secure {
                WINHTTP_FLAG_SECURE
            } else {
                WINHTTP_OPEN_REQUEST_FLAGS(0)
            };
            let handle = Handle::new(WinHttpOpenRequest(
                connection.0,
                w!("GET"),
                PCWSTR(p.path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                ptr::null(),
                flags,
            ))?;
            let disabled = (WINHTTP_DISABLE_COOKIES
                | WINHTTP_DISABLE_REDIRECTS
                | WINHTTP_DISABLE_AUTHENTICATION)
                .to_ne_bytes();
            WinHttpSetOption(
                Some(handle.0),
                WINHTTP_OPTION_DISABLE_FEATURE,
                Some(&disabled),
            )
            .map_err(|e| e.to_string())?;
            WinHttpSendRequest(handle.0, None, None, 0, 0, 0).map_err(|e| e.to_string())?;
            WinHttpReceiveResponse(handle.0, ptr::null_mut()).map_err(|e| e.to_string())?;
            let mut status = 0u32;
            let mut len = 4u32;
            let mut index = 0u32;
            WinHttpQueryHeaders(
                handle.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some((&mut status as *mut u32).cast()),
                &mut len,
                &mut index,
            )
            .map_err(|e| e.to_string())?;
            if (300..400).contains(&status) {
                return Err(format!(
                    "HTTP redirect {status} refused; location: {}",
                    header(&handle, WINHTTP_QUERY_LOCATION).unwrap_or_default()
                ));
            }
            let content_type = header(&handle, WINHTTP_QUERY_CONTENT_TYPE);
            let mut body = Vec::new();
            loop {
                let mut buffer = [0u8; 8192];
                let mut read = 0u32;
                WinHttpReadData(
                    handle.0,
                    buffer.as_mut_ptr().cast(),
                    buffer.len() as u32,
                    &mut read,
                )
                .map_err(|e| e.to_string())?;
                if read == 0 {
                    break;
                }
                if body.len() + read as usize > request.max_bytes {
                    return Err("response exceeds max_bytes".into());
                }
                body.extend_from_slice(&buffer[..read as usize]);
            }
            Ok(FetchResponse {
                url: request.url.clone(),
                status,
                content_type,
                body: String::from_utf8(body)
                    .map_err(|_| "response body is not UTF-8".to_string())?,
                fetched_at_ms: now_ms()?,
                cached: false,
            })
        }
    }
}
