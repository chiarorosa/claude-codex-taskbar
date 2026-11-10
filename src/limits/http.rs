//! Minimal JSON GET used by the providers, on the OS's WinHTTP (#83).
//!
//! WinHTTP replaces ureq + native-tls: TLS is the same Schannel with the Windows certificate
//! store, and the binary loses ~375 KB. Every guarantee the ureq client had is kept and tested:
//! HTTPS only, no redirects, certificate validation always on, a 10 s budget per request, the
//! same status mapping, error text without headers, the same User-Agent and the same
//! environment proxy rules (`ALL_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY`, `NO_PROXY`).

use super::FetchError;
use std::time::{Duration, Instant};
use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_NAMED_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY,
    WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH, WINHTTP_DISABLE_COOKIES, WINHTTP_DISABLE_REDIRECTS,
    WINHTTP_FLAG_SECURE, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_AUTOLOGON_POLICY, WINHTTP_OPTION_DISABLE_FEATURE,
    WINHTTP_OPTION_PROXY_PASSWORD, WINHTTP_OPTION_PROXY_USERNAME,
    WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_OPTION_SECURE_PROTOCOLS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR};

const TIMEOUT: Duration = Duration::from_secs(10);
/// A usage response is a few KB; anything this big is not one.
const MAX_BODY: usize = 1 << 20;
const USER_AGENT: &str = concat!("claude-codex-taskbar/", env!("CARGO_PKG_VERSION"));

/// 2xx is success, 401/403 mean expired credentials, anything else (3xx included) is an error.
fn map_status(status: u16) -> Result<(), FetchError> {
    match status {
        200..=299 => Ok(()),
        401 | 403 => Err(FetchError::Expired),
        _ => Err(FetchError::BadResponse(format!("HTTP {status}"))),
    }
}

/// Where a request goes, split from its URL.
#[derive(Debug, PartialEq, Eq)]
struct Target {
    secure: bool,
    host: String,
    port: u16,
    /// Path and query, always starting with `/`.
    path: String,
}

/// Parses `https://host[:port][/path][?query]`. Plain `http://` only when `allow_plain` (tests).
/// User info (`user@host`) is refused so a URL can never carry credentials.
fn parse_target(url: &str, allow_plain: bool) -> Result<Target, FetchError> {
    let bad = |why: &str| FetchError::Network(format!("invalid URL: {why}"));
    if url
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return Err(bad("characters"));
    }
    let (scheme, rest) = url.split_once("://").ok_or_else(|| bad("no scheme"))?;
    let secure = if scheme.eq_ignore_ascii_case("https") {
        true
    } else if scheme.eq_ignore_ascii_case("http") && allow_plain {
        false
    } else {
        return Err(bad("only HTTPS is allowed"));
    };
    if rest.contains('#') {
        return Err(bad("fragment"));
    }
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    if authority.contains('@') {
        return Err(bad("credentials in URL"));
    }
    let (host, port) = match authority.split_once(':') {
        Some((host, port)) => {
            let port: u16 = port.parse().map_err(|_| bad("port"))?;
            (host, port)
        }
        None => (authority, if secure { 443 } else { 80 }),
    };
    let host_ok = |c: char| c.is_ascii_alphanumeric() || c == '.' || c == '-';
    if host.is_empty() || port == 0 || !host.chars().all(host_ok) {
        return Err(bad("host"));
    }
    let path = match path.chars().next() {
        None => "/".to_string(),
        Some('?') => format!("/{path}"),
        Some(_) => path.to_string(),
    };
    Ok(Target {
        secure,
        host: host.to_string(),
        port,
        path,
    })
}

/// An HTTP proxy taken from the environment, the way ureq read it.
#[derive(PartialEq, Eq)]
struct Proxy {
    /// `host:port` for WinHTTP.
    server: String,
    user: Option<String>,
    pass: Option<String>,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("server", &self.server)
            .field("user", &self.user)
            .field("pass", &self.pass.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What a proxy variable holds.
enum ProxyValue {
    Http(Proxy),
    /// A proxy URI with a scheme WinHTTP cannot use (`https://`, `socks5://`, ...).
    Unsupported,
}

/// The proxy for `host`: the first parsable `ALL_PROXY`, `HTTPS_PROXY`, `HTTP_PROXY` (each
/// upper- then lowercase), unless `NO_PROXY` / `no_proxy` matches the host. Only HTTP proxies
/// (`http://` or no scheme) are supported; other schemes are skipped like an invalid value.
fn env_proxy(
    host: &str,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<Proxy>, FetchError> {
    const TRY_ENV: [&str; 6] = [
        "ALL_PROXY",
        "all_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
    ];
    let Some(value) = TRY_ENV
        .iter()
        .find_map(|name| var(name).and_then(|v| parse_proxy(&v)))
    else {
        return Ok(None);
    };
    let no_proxy = var("NO_PROXY").or_else(|| var("no_proxy"));
    if no_proxy.is_some_and(|list| bypasses(&list, host)) {
        return Ok(None);
    }
    match value {
        ProxyValue::Http(proxy) => Ok(Some(proxy)),
        // Fail closed: going direct would skip a proxy the user relies on.
        ProxyValue::Unsupported => Err(FetchError::Network("unsupported proxy".into())),
    }
}

/// `[http://][user[:pass]@]host[:port][/]`; port 80 when missing. `None` for a value that is
/// not a proxy URI at all, which is skipped like ureq did.
fn parse_proxy(value: &str) -> Option<ProxyValue> {
    let value = value.trim();
    let rest = match value.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some((scheme, rest)) => {
            let known = ["https", "socks", "socks4", "socks4a", "socks5", "socks5h"];
            let is_uri = !rest.is_empty() && known.iter().any(|k| scheme.eq_ignore_ascii_case(k));
            return is_uri.then_some(ProxyValue::Unsupported);
        }
        None => value,
    };
    let authority = rest.split('/').next()?;
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((userinfo, hostport)) => (Some(userinfo), hostport),
        None => (None, authority),
    };
    let (host, port) = match hostport.split_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok().filter(|p| *p != 0)?),
        None => (hostport, 80),
    };
    if host.is_empty() {
        return None;
    }
    let (user, pass) = match userinfo.map(|u| u.split_once(':').unwrap_or((u, ""))) {
        Some((user, pass)) if !user.is_empty() => (
            Some(percent_decode(user)),
            (!pass.is_empty()).then(|| percent_decode(pass)),
        ),
        _ => (None, None),
    };
    Some(ProxyValue::Http(Proxy {
        server: format!("{host}:{port}"),
        user,
        pass,
    }))
}

/// Decodes `%XX` escapes; anything malformed is kept as written.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        match (
            bytes[i],
            bytes.get(i + 1).and_then(|b| hex(*b)),
            bytes.get(i + 2).and_then(|b| hex(*b)),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (b, _, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// `NO_PROXY` entries as ureq matched them: `*`, `*suffix` / `.suffix`, `prefix*` / `prefix.`,
/// or an exact host; comma separated, case-insensitive.
fn bypasses(no_proxy: &str, host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    no_proxy
        .split(',')
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty())
        .any(|e| {
            if e == "*" {
                true
            } else if let Some(suffix) = e.strip_prefix('*') {
                host.ends_with(suffix)
            } else if e.starts_with('.') {
                host.ends_with(&e)
            } else if let Some(prefix) = e.strip_suffix('*') {
                host.starts_with(prefix)
            } else if e.ends_with('.') {
                host.starts_with(&e)
            } else {
                host == e
            }
        })
}

/// The request headers block, refusing CR/LF in names or values (header injection).
fn header_block(headers: &[(&str, &str)]) -> Result<String, FetchError> {
    let mut out = String::new();
    for (name, value) in headers {
        // Names are RFC 9110 tokens; values are visible ASCII, space or tab.
        let token = |c: char| c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c);
        let bad_name = name.is_empty() || !name.chars().all(token);
        let bad_value = !value.chars().all(|c| c == '\t' || (' '..='~').contains(&c));
        if bad_name || bad_value {
            return Err(FetchError::Network("invalid request header".into()));
        }
        out.push_str(name);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    Ok(out)
}

/// Appends a body chunk, failing once the body would pass `cap`.
fn push_capped(body: &mut Vec<u8>, chunk: &[u8], cap: usize) -> Result<(), FetchError> {
    if body.len() + chunk.len() > cap {
        return Err(FetchError::BadResponse("response too large".into()));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

/// How a request runs. Production uses [`Policy::production`]; tests relax only what they name.
struct Policy {
    allow_plain: bool,
    timeout: Duration,
    proxy: Option<Proxy>,
}

impl Policy {
    fn production(host: &str) -> Result<Self, FetchError> {
        Ok(Self {
            allow_plain: false,
            timeout: TIMEOUT,
            proxy: env_proxy(host, |k| std::env::var(k).ok())?,
        })
    }
}

/// GETs `url` and parses JSON. 401/403 map to `Expired`. Error text never includes headers.
pub fn get_json(url: &str, headers: &[(&str, &str)]) -> Result<serde_json::Value, FetchError> {
    let host = parse_target(url, false)?.host;
    fetch(url, headers, &Policy::production(&host)?)
}

/// A WinHTTP handle, closed on drop.
struct Handle(*mut core::ffi::c_void);

impl Handle {
    fn new(raw: *mut core::ffi::c_void) -> Result<Self, FetchError> {
        if raw.is_null() {
            Err(net_error(&windows::core::Error::from_thread()))
        } else {
            Ok(Self(raw))
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle came from WinHTTP and is closed exactly once.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// Short, header-free text for a WinHTTP failure, shown in the tooltip.
fn net_error(err: &windows::core::Error) -> FetchError {
    let code = (err.code().0 as u32) & 0xFFFF;
    let text = match code {
        12002 => "timed out".to_string(),
        12007 => "host not found".to_string(),
        12029 | 12030 => "could not connect".to_string(),
        12037 | 12038 | 12044 | 12045 | 12057 | 12175 => "TLS or certificate error".to_string(),
        12152 => "invalid server response".to_string(),
        _ => format!("network error {code}"),
    };
    FetchError::Network(text)
}

fn set_u32(handle: &Handle, option: u32, value: u32) -> windows::core::Result<()> {
    // SAFETY: `handle` is live and a DWORD option takes a 4-byte buffer.
    unsafe { WinHttpSetOption(Some(handle.0), option, Some(&value.to_ne_bytes())) }
}

/// Sets a string option. WinHTTP takes their length in characters, not bytes.
fn set_wide(handle: &Handle, option: u32, value: &str) -> windows::core::Result<()> {
    // NUL-terminated: WinHTTP reads these options up to the terminator, whatever the length says
    // (without it, a proxy password picked up stray bytes past the end of the buffer).
    let wide: Vec<u16> = value.encode_utf16().chain(Some(0)).collect();
    let chars = wide.len() - 1;
    // SAFETY: the slice spans `chars` bytes of the `2 * wide.len()`-byte buffer, which hands
    // WinHTTP the UTF-16 pointer with the string's length in characters, as it expects.
    let buf = unsafe { std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), chars) };
    // SAFETY: `handle` is live and `wide` outlives the call.
    unsafe { WinHttpSetOption(Some(handle.0), option, Some(buf)) }
}

/// Spends what is left of the request's budget on the next WinHTTP step. WinHTTP rounds these
/// up to ~4 s steps, so a server that accepts and stays silent ends the 10 s budget by ~12 s.
fn arm_timeouts(request: &Handle, deadline: Instant) -> Result<(), FetchError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(FetchError::Network("timed out".into()));
    }
    let ms = left.as_millis().clamp(1, i32::MAX as u128) as i32;
    // SAFETY: `request` is a live request handle.
    unsafe { WinHttpSetTimeouts(request.0, ms, ms, ms, ms) }.map_err(|e| net_error(&e))?;
    // Waiting for the response headers has its own timeout (90 s by default).
    set_u32(request, WINHTTP_OPTION_RECEIVE_RESPONSE_TIMEOUT, ms as u32).map_err(|e| net_error(&e))
}

fn fetch(
    url: &str,
    headers: &[(&str, &str)],
    policy: &Policy,
) -> Result<serde_json::Value, FetchError> {
    let deadline = Instant::now() + policy.timeout;
    let target = parse_target(url, policy.allow_plain)?;
    let block = format!("Accept: */*\r\n{}", header_block(headers)?);
    let block: Vec<u16> = block.encode_utf16().collect();
    let net = |e: windows::core::Error| net_error(&e);

    let (access, proxy_name) = match &policy.proxy {
        Some(p) => (
            WINHTTP_ACCESS_TYPE_NAMED_PROXY,
            HSTRING::from(p.server.as_str()),
        ),
        None => (WINHTTP_ACCESS_TYPE_NO_PROXY, HSTRING::new()),
    };
    let proxy_ptr = if proxy_name.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(proxy_name.as_ptr())
    };
    // SAFETY: every string outlives the call that receives it; handles are owned by `Handle`.
    let session = Handle::new(unsafe {
        WinHttpOpen(
            &HSTRING::from(USER_AGENT),
            access,
            proxy_ptr,
            PCWSTR::null(),
            0,
        )
    })?;
    arm_timeouts(&session, deadline)?;
    set_u32(
        &session,
        WINHTTP_OPTION_REDIRECT_POLICY,
        WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
    )
    .map_err(net)?;
    // TLS 1.2 and 1.3; older Windows 10 builds reject the 1.3 flag, so fall back to 1.2 only.
    let tls12 = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
    if set_u32(
        &session,
        WINHTTP_OPTION_SECURE_PROTOCOLS,
        tls12 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
    )
    .is_err()
    {
        set_u32(&session, WINHTTP_OPTION_SECURE_PROTOCOLS, tls12).map_err(net)?;
    }
    // SAFETY: as above.
    let connect = Handle::new(unsafe {
        WinHttpConnect(
            session.0,
            &HSTRING::from(target.host.as_str()),
            target.port,
            0,
        )
    })?;
    let flags = if target.secure {
        WINHTTP_FLAG_SECURE
    } else {
        WINHTTP_OPEN_REQUEST_FLAGS(0)
    };
    // SAFETY: as above; a null accept-types list adds no Accept header of its own.
    let request = Handle::new(unsafe {
        WinHttpOpenRequest(
            connect.0,
            &HSTRING::from("GET"),
            &HSTRING::from(target.path.as_str()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        )
    })?;
    // No cookies, no redirects (on top of the session policy), and never the Windows logon
    // credentials. Certificate checks are WinHTTP's default and are never relaxed.
    set_u32(
        &request,
        WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_REDIRECTS,
    )
    .map_err(net)?;
    set_u32(
        &request,
        WINHTTP_OPTION_AUTOLOGON_POLICY,
        WINHTTP_AUTOLOGON_SECURITY_LEVEL_HIGH,
    )
    .map_err(net)?;
    if let Some(p) = &policy.proxy {
        if let Some(user) = &p.user {
            set_wide(&request, WINHTTP_OPTION_PROXY_USERNAME, user).map_err(net)?;
        }
        if let Some(pass) = &p.pass {
            set_wide(&request, WINHTTP_OPTION_PROXY_PASSWORD, pass).map_err(net)?;
        }
    }

    arm_timeouts(&request, deadline)?;
    // SAFETY: `block` outlives the call; no request body.
    unsafe { WinHttpSendRequest(request.0, Some(&block), None, 0, 0, 0) }.map_err(net)?;
    arm_timeouts(&request, deadline)?;
    // SAFETY: `request` is live.
    unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }.map_err(net)?;

    let mut status = 0u32;
    let mut len = size_of::<u32>() as u32;
    // SAFETY: a numeric query writes one DWORD into `status`.
    unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&raw mut status).cast()),
            &mut len,
            std::ptr::null_mut(),
        )
    }
    .map_err(net)?;
    map_status(u16::try_from(status).unwrap_or(0))?;

    let mut body = Vec::new();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        arm_timeouts(&request, deadline)?;
        let mut n = 0u32;
        // SAFETY: WinHTTP writes at most `buf.len()` bytes into `buf`.
        unsafe { WinHttpReadData(request.0, buf.as_mut_ptr().cast(), buf.len() as u32, &mut n) }
            .map_err(net)?;
        if n == 0 {
            break;
        }
        push_capped(&mut body, &buf[..n as usize], MAX_BODY)?;
    }
    serde_json::from_slice(&body).map_err(|_| FetchError::BadResponse("invalid JSON".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Instant;

    const SECRET: &str = "acct-secret-123";

    #[test]
    fn redirect_statuses_are_bad_response() {
        for status in [300, 301, 302, 303, 307, 308] {
            assert_eq!(
                map_status(status),
                Err(FetchError::BadResponse(format!("HTTP {status}"))),
                "status {status}"
            );
        }
    }

    #[test]
    fn auth_statuses_are_expired() {
        assert_eq!(map_status(401), Err(FetchError::Expired));
        assert_eq!(map_status(403), Err(FetchError::Expired));
    }

    #[test]
    fn success_statuses_are_ok() {
        assert_eq!(map_status(200), Ok(()));
        assert_eq!(map_status(299), Ok(()));
    }

    #[test]
    fn other_statuses_are_bad_response() {
        for status in [100, 400, 404, 429, 500, 503] {
            assert_eq!(
                map_status(status),
                Err(FetchError::BadResponse(format!("HTTP {status}")))
            );
        }
    }

    fn target(secure: bool, host: &str, port: u16, path: &str) -> Target {
        Target {
            secure,
            host: host.into(),
            port,
            path: path.into(),
        }
    }

    #[test]
    fn targets_parse_host_port_and_path() {
        let cases = [
            (
                "https://api.anthropic.com/api/oauth/usage",
                target(true, "api.anthropic.com", 443, "/api/oauth/usage"),
            ),
            (
                "https://chatgpt.com/backend-api/wham/usage?x=1",
                target(true, "chatgpt.com", 443, "/backend-api/wham/usage?x=1"),
            ),
            ("https://h.example", target(true, "h.example", 443, "/")),
            (
                "https://h.example:8443/a",
                target(true, "h.example", 8443, "/a"),
            ),
            ("HTTPS://H.example/a", target(true, "H.example", 443, "/a")),
            ("https://h.example?q", target(true, "h.example", 443, "/?q")),
        ];
        for (url, want) in cases {
            assert_eq!(parse_target(url, false), Ok(want), "{url}");
        }
        assert_eq!(
            parse_target("http://127.0.0.1:9/u", true),
            Ok(target(false, "127.0.0.1", 9, "/u"))
        );
        assert_eq!(
            parse_target("http://127.0.0.1/u", true),
            Ok(target(false, "127.0.0.1", 80, "/u"))
        );
    }

    #[test]
    fn bad_targets_are_network_errors() {
        for url in [
            "http://api.anthropic.com/x",
            "ftp://h.example/x",
            "api.anthropic.com/x",
            "https://",
            "https:///x",
            "https://h.example:/x",
            "https://h.example:99999/x",
            "https://h.example:8a/x",
            "https://user:pw@h.example/x",
            "https://evil@h.example/x",
            "https://h.example#frag",
            "https://[::1]/x",
            "https://evil\\x/y",
            "https://h%40x/y",
            "https://h.ex ample/x",
            " https://h.example/x",
            "https://h.example/x ",
            "https://h.example/a b",
            "https://h.example/\r\nX: 1",
        ] {
            assert!(
                matches!(parse_target(url, false), Err(FetchError::Network(_))),
                "{url}"
            );
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn proxy(server: &str) -> Proxy {
        Proxy {
            server: server.into(),
            user: None,
            pass: None,
        }
    }

    #[test]
    fn env_proxy_follows_urequs_order() {
        let host = "api.anthropic.com";
        assert_eq!(env_proxy(host, vars(&[])), Ok(None));
        assert_eq!(
            env_proxy(host, vars(&[("HTTP_PROXY", "http://p:3128")])),
            Ok(Some(proxy("p:3128")))
        );
        assert_eq!(
            env_proxy(host, vars(&[("http_proxy", "p:8080")])),
            Ok(Some(proxy("p:8080")))
        );
        assert_eq!(
            env_proxy(
                host,
                vars(&[
                    ("HTTP_PROXY", "http://c:1"),
                    ("https_proxy", "http://b:1"),
                    ("ALL_PROXY", "http://a:1"),
                ])
            ),
            Ok(Some(proxy("a:1")))
        );
        assert_eq!(
            env_proxy(
                host,
                vars(&[("HTTPS_PROXY", "http://b:1"), ("all_proxy", "http://a:1")])
            ),
            Ok(Some(proxy("a:1")))
        );
    }

    #[test]
    fn env_proxy_defaults_port_and_reads_credentials() {
        let host = "chatgpt.com";
        assert_eq!(
            env_proxy(host, vars(&[("HTTPS_PROXY", "http://proxy.corp")])),
            Ok(Some(proxy("proxy.corp:80")))
        );
        assert_eq!(
            env_proxy(
                host,
                vars(&[("HTTPS_PROXY", "http://me:s3cret@proxy.corp:8080/")])
            ),
            Ok(Some(Proxy {
                server: "proxy.corp:8080".into(),
                user: Some("me".into()),
                pass: Some("s3cret".into()),
            }))
        );
        assert_eq!(
            env_proxy(
                host,
                vars(&[
                    ("ALL_PROXY", "socks5://s:1080"),
                    ("HTTP_PROXY", "http://p:3128")
                ])
            ),
            Err(FetchError::Network("unsupported proxy".into()))
        );
        assert_eq!(env_proxy(host, vars(&[("HTTP_PROXY", "")])), Ok(None));
    }

    #[test]
    fn an_unsupported_proxy_fails_closed_instead_of_going_direct() {
        let host = "api.anthropic.com";
        for value in ["socks5://127.0.0.1:1080", "https://p:443", "SOCKS4://p:1"] {
            assert_eq!(
                env_proxy(host, vars(&[("ALL_PROXY", value)])),
                Err(FetchError::Network("unsupported proxy".into())),
                "{value}"
            );
        }
        // A host the user exempted still goes direct.
        assert_eq!(
            env_proxy(
                host,
                vars(&[
                    ("ALL_PROXY", "socks5://s:1"),
                    ("NO_PROXY", ".anthropic.com")
                ])
            ),
            Ok(None)
        );
        // A value that is not a proxy URI at all is skipped, as ureq did.
        assert_eq!(
            env_proxy(
                host,
                vars(&[("ALL_PROXY", "http://"), ("HTTP_PROXY", "http://p:1")])
            ),
            Ok(Some(proxy("p:1")))
        );
    }

    #[test]
    fn proxy_credentials_are_percent_decoded_and_never_printed() {
        let got = env_proxy(
            "chatgpt.com",
            vars(&[("HTTPS_PROXY", "http://me%40corp:p%40ss%3Aw0rd@proxy:8080")]),
        )
        .unwrap()
        .unwrap();
        assert_eq!(got.user.as_deref(), Some("me@corp"));
        assert_eq!(got.pass.as_deref(), Some("p@ss:w0rd"));
        let shown = format!("{got:?}");
        assert!(!shown.contains("p@ss"), "{shown}");
        assert!(shown.contains("proxy:8080"), "{shown}");
        for value in ["http://:pw@proxy:1", "http://@proxy:1"] {
            assert_eq!(
                env_proxy("chatgpt.com", vars(&[("HTTP_PROXY", value)])),
                Ok(Some(proxy("proxy:1"))),
                "{value}"
            );
        }
    }

    #[test]
    fn no_proxy_matches_like_urequ() {
        let cases = [
            ("*", "api.anthropic.com", true),
            ("api.anthropic.com", "api.anthropic.com", true),
            ("API.Anthropic.com", "api.anthropic.com", true),
            ("anthropic.com", "api.anthropic.com", false),
            (".anthropic.com", "api.anthropic.com", true),
            ("*.anthropic.com", "api.anthropic.com", true),
            ("*anthropic.com", "api.anthropic.com", true),
            ("api.*", "api.anthropic.com", true),
            ("api.", "api.anthropic.com", true),
            ("localhost, chatgpt.com", "chatgpt.com", true),
            ("localhost,127.0.0.1", "chatgpt.com", false),
            ("", "chatgpt.com", false),
        ];
        for (no_proxy, host, want) in cases {
            assert_eq!(bypasses(no_proxy, host), want, "{no_proxy:?} {host}");
        }
        assert_eq!(
            env_proxy(
                "chatgpt.com",
                vars(&[("HTTP_PROXY", "http://p:1"), ("no_proxy", "chatgpt.com")])
            ),
            Ok(None)
        );
        assert_eq!(
            env_proxy(
                "chatgpt.com",
                vars(&[("HTTP_PROXY", "http://p:1"), ("NO_PROXY", "other.com")])
            ),
            Ok(Some(proxy("p:1")))
        );
    }

    #[test]
    fn headers_are_joined_and_injection_is_refused() {
        assert_eq!(
            header_block(&[("Authorization", "Bearer x"), ("anthropic-beta", "b")]),
            Ok("Authorization: Bearer x\r\nanthropic-beta: b\r\n".into())
        );
        for bad in [
            [("A", "x\r\nEvil: 1")],
            [("A", "x\nEvil: 1")],
            [("A\r\nEvil", "x")],
            [("", "x")],
            [("A", "x\0y")],
            [("A", "x\u{7f}")],
            [("A B", "x")],
            [("A:", "x")],
        ] {
            let err = header_block(&bad).unwrap_err();
            assert!(matches!(err, FetchError::Network(_)), "{bad:?}");
            assert!(!format!("{err:?}").contains('x'), "{err:?}");
        }
    }

    #[test]
    fn production_policy_is_https_only_with_a_10_second_budget() {
        let p = Policy::production("api.anthropic.com").unwrap();
        assert!(!p.allow_plain);
        assert_eq!(p.timeout, Duration::from_secs(10));
    }

    #[test]
    fn certificate_checks_are_never_relaxed() {
        // The only way to weaken WinHTTP's certificate validation is a security-flags option.
        let source = include_str!("http.rs");
        for needle in [["SECURITY", "_FLAG"].concat(), ["IGNORE", "_CERT"].concat()] {
            assert!(!source.contains(&needle), "{needle} found in http.rs");
        }
    }

    #[test]
    fn body_is_capped() {
        let mut body = Vec::new();
        assert_eq!(push_capped(&mut body, b"abc", 5), Ok(()));
        assert_eq!(push_capped(&mut body, b"de", 5), Ok(()));
        assert_eq!(
            push_capped(&mut body, b"f", 5),
            Err(FetchError::BadResponse("response too large".into()))
        );
    }

    /// The production policy with only plain HTTP allowed, so a local server can answer.
    fn local(timeout: Duration) -> Policy {
        Policy {
            allow_plain: true,
            timeout,
            proxy: None,
        }
    }

    /// Answers the first connection with `response` and returns the raw request it read.
    fn serve_once(listener: TcpListener, response: String) -> thread::JoinHandle<String> {
        thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = sock.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                req.extend_from_slice(&buf[..n]);
            }
            sock.write_all(response.as_bytes()).unwrap();
            String::from_utf8_lossy(&req).into_owned()
        })
    }

    fn assert_no_connection(listener: &TcpListener) {
        listener.set_nonblocking(true).unwrap();
        match listener.accept() {
            Err(e) => assert_eq!(e.kind(), ErrorKind::WouldBlock),
            Ok(_) => panic!("a connection reached the listener"),
        }
    }

    fn reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn plain_http_is_refused_before_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/usage", listener.local_addr().unwrap());

        let err = get_json(&url, &[("ChatGPT-Account-Id", SECRET)]).unwrap_err();

        assert!(matches!(err, FetchError::Network(_)), "{err:?}");
        assert!(!format!("{err:?}").contains(SECRET));
        assert_no_connection(&listener);
    }

    #[test]
    fn redirect_is_not_followed_and_header_does_not_leak() {
        // Cross-host target: a second listener that must never be contacted.
        let target = TcpListener::bind("127.0.0.1:0").unwrap();
        let target_url = format!("http://{}/stolen", target.local_addr().unwrap());
        let origin = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin_url = format!("http://{}/usage", origin.local_addr().unwrap());
        let server = serve_once(
            origin,
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
        );

        let err = fetch(
            &origin_url,
            &[("ChatGPT-Account-Id", SECRET)],
            &local(TIMEOUT),
        )
        .unwrap_err();

        assert_eq!(err, FetchError::BadResponse("HTTP 302".into()));
        assert!(!format!("{err:?}").contains(SECRET));
        let seen = server.join().unwrap();
        assert!(
            seen.contains(SECRET),
            "origin should get the header: {seen}"
        );
        assert_no_connection(&target);
    }

    #[test]
    fn json_is_parsed_and_request_carries_headers_and_user_agent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/usage?x=1", listener.local_addr().unwrap());
        let server = serve_once(
            listener,
            reply("200 OK", r#"{"five_hour":{"utilization":12}}"#),
        );

        let v = fetch(&url, &[("Authorization", "Bearer t0k")], &local(TIMEOUT)).unwrap();

        assert_eq!(v["five_hour"]["utilization"], 12);
        let seen = server.join().unwrap();
        assert!(
            seen.starts_with("GET /api/usage?x=1 HTTP/1.1\r\n"),
            "{seen}"
        );
        assert!(seen.contains("Authorization: Bearer t0k\r\n"), "{seen}");
        assert!(seen.contains("Accept: */*\r\n"), "{seen}");
        assert!(
            seen.contains(&format!("User-Agent: {USER_AGENT}\r\n")),
            "{seen}"
        );
        assert!(!seen.to_ascii_lowercase().contains("cookie"), "{seen}");
    }

    /// Reads one request head (up to the blank line) from `sock`.
    fn read_head(sock: &mut std::net::TcpStream) -> String {
        let mut req = Vec::new();
        let mut buf = [0u8; 1024];
        while !req.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = sock.read(&mut buf).unwrap_or(0);
            if n == 0 {
                break;
            }
            req.extend_from_slice(&buf[..n]);
        }
        String::from_utf8_lossy(&req).into_owned()
    }

    #[test]
    fn https_goes_through_the_proxy_with_basic_auth_and_no_secret_in_the_clear() {
        let proxy_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let server = proxy_listener.local_addr().unwrap().to_string();
        let seen = thread::spawn(move || {
            let mut heads = Vec::new();
            // First CONNECT: ask for credentials.
            let (mut sock, _) = proxy_listener.accept().unwrap();
            heads.push(read_head(&mut sock));
            sock.write_all(
                b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"corp\"\r\nContent-Length: 0\r\n\r\n",
            )
            .unwrap();
            // The retry may reuse the connection or open a new one.
            proxy_listener.set_nonblocking(true).unwrap();
            let again = read_head(&mut sock);
            if again.is_empty() {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    match proxy_listener.accept() {
                        Ok((mut s2, _)) => {
                            s2.set_nonblocking(false).unwrap();
                            heads.push(read_head(&mut s2));
                            break;
                        }
                        Err(_) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => break,
                    }
                }
            } else {
                heads.push(again);
            }
            heads
        });

        let policy = Policy {
            allow_plain: false,
            timeout: Duration::from_secs(5),
            proxy: Some(Proxy {
                server,
                user: Some("me".into()),
                pass: Some("s3cret".into()),
            }),
        };
        // The proxy never completes the tunnel, so the request fails; what matters is what
        // the proxy saw.
        let err = fetch(
            "https://usage.example/api",
            &[("Authorization", SECRET), ("ChatGPT-Account-Id", SECRET)],
            &policy,
        )
        .unwrap_err();
        let heads = seen.join().unwrap();

        assert!(!format!("{err:?}").contains(SECRET), "{err:?}");
        assert!(
            heads[0].starts_with("CONNECT usage.example:443 HTTP/1.1\r\n"),
            "{heads:?}"
        );
        // base64("me:s3cret")
        assert!(
            heads
                .iter()
                .any(|h| h.contains("Proxy-Authorization: Basic bWU6czNjcmV0\r\n")),
            "{heads:?}"
        );
        assert!(heads.iter().all(|h| !h.contains(SECRET)), "{heads:?}");
    }

    #[test]
    fn auth_failure_and_bad_json_map_like_before() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/u", listener.local_addr().unwrap());
        let server = serve_once(listener, reply("401 Unauthorized", "{}"));
        assert_eq!(fetch(&url, &[], &local(TIMEOUT)), Err(FetchError::Expired));
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/u", listener.local_addr().unwrap());
        let server = serve_once(listener, reply("200 OK", "not json"));
        assert_eq!(
            fetch(&url, &[], &local(TIMEOUT)),
            Err(FetchError::BadResponse("invalid JSON".into()))
        );
        server.join().unwrap();
    }

    #[test]
    fn a_server_that_never_answers_times_out_within_the_budget() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/u", listener.local_addr().unwrap());
        // Accepts, reads the request and then stays silent until the test is done.
        let (done, wait) = std::sync::mpsc::channel::<()>();
        let hold = thread::spawn(move || {
            let (sock, _) = listener.accept().unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(30));
            drop(sock);
        });

        // WinHTTP rounds synchronous timeouts up to ~4 s steps, so a silent server ends the
        // request between the budget and the budget plus one step (10 s → at most ~12 s).
        let budget = Duration::from_secs(3);
        let started = Instant::now();
        let err = fetch(&url, &[("Authorization", SECRET)], &local(budget)).unwrap_err();
        let took = started.elapsed();
        done.send(()).unwrap();
        hold.join().unwrap();

        assert!(matches!(err, FetchError::Network(_)), "{err:?}");
        assert!(!format!("{err:?}").contains(SECRET));
        assert!(
            took >= budget - Duration::from_millis(500)
                && took < budget + Duration::from_millis(4500),
            "{took:?} {err:?}"
        );
    }
}
