//! Client-side TLS for compiled Raven programs, backing `std/tls`.
//!
//! Mirrors `std/net`'s C ABI conventions: each connection and each config lives
//! in a global registry keyed by an opaque `i64` id (0 means failure), inputs
//! and outputs cross as `object::String` byte buffers, and failures are left in
//! a thread-local last-error slot the Raven wrapper reads after every call.
//!
//! TLS is `rustls` with the `ring` crypto provider and the bundled Mozilla root
//! store (`webpki-roots`). A connection is a `StreamOwned<ClientConnection,
//! TcpStream>` behind its own mutex, since a TLS session is stateful and cannot
//! be cloned per-read the way a raw socket fd is. Every blocking handshake and
//! read/write is wrapped in `gc::blocking`, the same bracket `std/net` uses, so
//! a goroutine waiting on TLS I/O cooperates with the elastic worker pool and
//! parks at a GC safepoint.

use crate::object;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::slice;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

type TlsStream = StreamOwned<ClientConnection, TcpStream>;
type TlsEntry = Arc<Mutex<TlsStream>>;

/// Live TLS connections, keyed by the id handed to Raven.
fn tls_registry() -> &'static Mutex<HashMap<i64, TlsEntry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, TlsEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A client configuration built up by the `raven_tls_config_*` setters and read
/// once when a connect turns it into an `Arc<ClientConfig>`.
#[derive(Default, Clone)]
struct TlsConfigSpec {
    extra_ca_pems: Vec<Vec<u8>>,
    client_cert_pem: Option<Vec<u8>>,
    client_key_pem: Option<Vec<u8>>,
    skip_verify: bool,
}

fn cfg_registry() -> &'static Mutex<HashMap<i64, TlsConfigSpec>> {
    static REGISTRY: OnceLock<Mutex<HashMap<i64, TlsConfigSpec>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Ids start at 1 so 0 stays free as the failure sentinel. Streams and configs
/// share the counter; they live in separate maps, so the values never collide.
fn next_id() -> i64 {
    static NEXT_ID: AtomicI64 = AtomicI64::new(1);
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

thread_local! {
    static TLS_LAST_ERROR: RefCell<std::string::String> =
        const { RefCell::new(std::string::String::new()) };
}

fn set_error(msg: std::string::String) {
    TLS_LAST_ERROR.with(|e| *e.borrow_mut() = msg);
}

fn clear_error() {
    TLS_LAST_ERROR.with(|e| e.borrow_mut().clear());
}

/// Read an incoming Raven `String` as an owned UTF-8 string, or `None` if it is
/// not valid UTF-8. A null or empty string reads as `""`.
fn read_str(s: *const object::String) -> Option<std::string::String> {
    if s.is_null() {
        return None;
    }
    let ptr = object::raven_string_bytes(s);
    let len = object::raven_string_len(s) as usize;
    if ptr.is_null() {
        return Some(std::string::String::new());
    }
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes).ok().map(|s| s.to_string())
}

fn empty_string() -> *mut object::String {
    object::raven_string_from_bytes(std::ptr::null(), 0)
}

#[no_mangle]
pub extern "C" fn raven_tls_last_error() -> *mut object::String {
    TLS_LAST_ERROR.with(|e| {
        let msg = e.borrow();
        object::raven_string_from_bytes(msg.as_ptr(), msg.len())
    })
}

#[no_mangle]
pub extern "C" fn raven_tls_config_new() -> i64 {
    let id = next_id();
    cfg_registry()
        .lock()
        .unwrap()
        .insert(id, TlsConfigSpec::default());
    clear_error();
    id
}

#[no_mangle]
pub extern "C" fn raven_tls_config_add_ca_file(cfg: i64, path: *const object::String) -> i64 {
    let path = match read_str(path) {
        Some(p) => p,
        None => {
            set_error("ca path is not valid UTF-8".to_string());
            return 0;
        }
    };
    let pem = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            set_error(format!("read CA file: {e}"));
            return 0;
        }
    };
    match cfg_registry().lock().unwrap().get_mut(&cfg) {
        Some(spec) => {
            spec.extra_ca_pems.push(pem);
            clear_error();
            1
        }
        None => {
            set_error("unknown tls config id".to_string());
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_config_client_cert(
    cfg: i64,
    cert_path: *const object::String,
    key_path: *const object::String,
) -> i64 {
    let cert_path = match read_str(cert_path) {
        Some(p) => p,
        None => {
            set_error("client cert path is not valid UTF-8".to_string());
            return 0;
        }
    };
    let key_path = match read_str(key_path) {
        Some(p) => p,
        None => {
            set_error("client key path is not valid UTF-8".to_string());
            return 0;
        }
    };
    let cert = match std::fs::read(&cert_path) {
        Ok(b) => b,
        Err(e) => {
            set_error(format!("read client cert: {e}"));
            return 0;
        }
    };
    let key = match std::fs::read(&key_path) {
        Ok(b) => b,
        Err(e) => {
            set_error(format!("read client key: {e}"));
            return 0;
        }
    };
    match cfg_registry().lock().unwrap().get_mut(&cfg) {
        Some(spec) => {
            spec.client_cert_pem = Some(cert);
            spec.client_key_pem = Some(key);
            clear_error();
            1
        }
        None => {
            set_error("unknown tls config id".to_string());
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_config_insecure_skip_verify(cfg: i64, on: bool) {
    if let Some(spec) = cfg_registry().lock().unwrap().get_mut(&cfg) {
        spec.skip_verify = on;
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_config_free(cfg: i64) {
    cfg_registry().lock().unwrap().remove(&cfg);
}

#[no_mangle]
pub extern "C" fn raven_tls_connect(
    addr: *const object::String,
    server_name: *const object::String,
    cfg: i64,
) -> i64 {
    let addr = match read_str(addr) {
        Some(a) => a,
        None => {
            set_error("addr is not valid UTF-8".to_string());
            return 0;
        }
    };
    let sni = match read_str(server_name) {
        Some(s) => s,
        None => {
            set_error("server_name is not valid UTF-8".to_string());
            return 0;
        }
    };
    let spec = if cfg == 0 {
        TlsConfigSpec::default()
    } else {
        match cfg_registry().lock().unwrap().get(&cfg) {
            Some(s) => s.clone(),
            None => {
                set_error("unknown tls config id".to_string());
                return 0;
            }
        }
    };
    let config = match build_client_config(&spec) {
        Ok(c) => c,
        Err(e) => {
            set_error(e);
            return 0;
        }
    };
    let server = match ServerName::try_from(sni.clone()) {
        Ok(s) => s,
        Err(_) => {
            set_error(format!("invalid server name: {sni}"));
            return 0;
        }
    };

    let result = crate::gc::blocking(|| -> Result<TlsStream, std::string::String> {
        let mut conn = ClientConnection::new(config, server).map_err(|e| e.to_string())?;
        let mut tcp = TcpStream::connect(&addr).map_err(|e| e.to_string())?;
        while conn.is_handshaking() {
            let (rd, wr) = conn.complete_io(&mut tcp).map_err(|e| e.to_string())?;
            if rd == 0 && wr == 0 && conn.is_handshaking() {
                return Err("tls handshake stalled".to_string());
            }
        }
        Ok(StreamOwned::new(conn, tcp))
    });

    match result {
        Ok(stream) => {
            let id = next_id();
            tls_registry()
                .lock()
                .unwrap()
                .insert(id, Arc::new(Mutex::new(stream)));
            clear_error();
            id
        }
        Err(e) => {
            set_error(e);
            0
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_read(stream_id: i64, max: i64) -> *mut object::String {
    if max < 0 {
        set_error("read size must be non-negative".to_string());
        return empty_string();
    }
    let cap = max as usize;
    let entry = match tls_registry().lock().unwrap().get(&stream_id).cloned() {
        Some(e) => e,
        None => {
            set_error("unknown tls stream id".to_string());
            return empty_string();
        }
    };
    let result = {
        let mut guard = entry.lock().unwrap();
        crate::gc::blocking(|| -> Result<Vec<u8>, std::string::String> {
            let mut buf: Vec<u8> = Vec::new();
            buf.try_reserve_exact(cap)
                .map_err(|_| "read size too large to allocate".to_string())?;
            buf.resize(cap, 0);
            let n = guard.read(&mut buf).map_err(|e| e.to_string())?;
            buf.truncate(n);
            Ok(buf)
        })
    };
    match result {
        Ok(bytes) => {
            clear_error();
            object::raven_string_from_bytes(bytes.as_ptr(), bytes.len())
        }
        Err(e) => {
            set_error(e);
            empty_string()
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_write(stream_id: i64, data: *const object::String) -> i64 {
    let ptr = object::raven_string_bytes(data);
    let len = object::raven_string_len(data) as usize;
    let bytes: &[u8] = if ptr.is_null() || len == 0 {
        &[]
    } else {
        unsafe { slice::from_raw_parts(ptr, len) }
    };
    let entry = match tls_registry().lock().unwrap().get(&stream_id).cloned() {
        Some(e) => e,
        None => {
            set_error("unknown tls stream id".to_string());
            return -1;
        }
    };
    let result = {
        let mut guard = entry.lock().unwrap();
        crate::gc::blocking(|| {
            guard
                .write_all(bytes)
                .and_then(|()| guard.flush())
                .map(|()| bytes.len() as i64)
                .map_err(|e| e.to_string())
        })
    };
    match result {
        Ok(n) => {
            clear_error();
            n
        }
        Err(e) => {
            set_error(e);
            -1
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_close(stream_id: i64) {
    let entry = tls_registry().lock().unwrap().remove(&stream_id);
    if let Some(entry) = entry {
        if let Ok(mut guard) = entry.lock() {
            guard.conn.send_close_notify();
            let _ = guard.flush();
            let _ = guard.sock.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_set_read_timeout_ms(stream_id: i64, ms: i64) {
    if let Some(entry) = tls_registry().lock().unwrap().get(&stream_id).cloned() {
        if let Ok(guard) = entry.lock() {
            let d = if ms <= 0 {
                None
            } else {
                Some(Duration::from_millis(ms as u64))
            };
            let _ = guard.sock.set_read_timeout(d);
        }
    }
}

#[no_mangle]
pub extern "C" fn raven_tls_set_write_timeout_ms(stream_id: i64, ms: i64) {
    if let Some(entry) = tls_registry().lock().unwrap().get(&stream_id).cloned() {
        if let Ok(guard) = entry.lock() {
            let d = if ms <= 0 {
                None
            } else {
                Some(Duration::from_millis(ms as u64))
            };
            let _ = guard.sock.set_write_timeout(d);
        }
    }
}

/// Install the `ring` crypto provider as the process default once, so the
/// `ClientConfig` builder and connections use it.
fn ensure_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

fn load_certs(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, std::string::String> {
    let mut rd = std::io::BufReader::new(pem);
    let mut out = Vec::new();
    for c in rustls_pemfile::certs(&mut rd) {
        out.push(c.map_err(|e| e.to_string())?);
    }
    if out.is_empty() {
        return Err("no certificates found in PEM".to_string());
    }
    Ok(out)
}

fn load_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, std::string::String> {
    let mut rd = std::io::BufReader::new(pem);
    match rustls_pemfile::private_key(&mut rd).map_err(|e| e.to_string())? {
        Some(k) => Ok(k),
        None => Err("no private key found in PEM".to_string()),
    }
}

fn build_client_config(spec: &TlsConfigSpec) -> Result<Arc<ClientConfig>, std::string::String> {
    ensure_provider();
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for pem in &spec.extra_ca_pems {
        for cert in load_certs(pem)? {
            roots.add(cert).map_err(|e| e.to_string())?;
        }
    }
    let builder = ClientConfig::builder();
    let config = if spec.skip_verify {
        let provider = rustls::crypto::CryptoProvider::get_default()
            .cloned()
            .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));
        let verifier = Arc::new(NoCertVerifier { provider });
        let b = builder
            .dangerous()
            .with_custom_certificate_verifier(verifier);
        finish_auth(b, spec)?
    } else {
        let b = builder.with_root_certificates(roots);
        finish_auth(b, spec)?
    };
    Ok(Arc::new(config))
}

fn finish_auth(
    b: rustls::ConfigBuilder<ClientConfig, rustls::client::WantsClientCert>,
    spec: &TlsConfigSpec,
) -> Result<ClientConfig, std::string::String> {
    match (&spec.client_cert_pem, &spec.client_key_pem) {
        (Some(cert), Some(key)) => {
            let certs = load_certs(cert)?;
            let key = load_key(key)?;
            b.with_client_auth_cert(certs, key)
                .map_err(|e| e.to_string())
        }
        _ => Ok(b.with_no_client_auth()),
    }
}

/// A verifier that accepts any certificate, for `insecure_skip_verify`. It still
/// checks the handshake signatures against the crypto provider, so the session
/// is encrypted; it just does not authenticate the peer. Development only.
#[derive(Debug)]
struct NoCertVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Registry and error paths that need neither the network nor the GC heap
    // (none of these construct an `object::String`). The handshake itself is
    // covered end to end by the std/tls examples.

    #[test]
    fn config_new_allocates_and_free_removes() {
        let id = raven_tls_config_new();
        assert!(id > 0, "config id must be a non-zero handle");
        assert!(cfg_registry().lock().unwrap().contains_key(&id));
        raven_tls_config_free(id);
        assert!(!cfg_registry().lock().unwrap().contains_key(&id));
    }

    #[test]
    fn config_setters_tolerate_unknown_ids() {
        // No-ops, must not panic on a stale or never-issued id.
        raven_tls_config_insecure_skip_verify(999_999, true);
        raven_tls_config_free(999_999);
    }

    #[test]
    fn connect_rejects_unreadable_addr() {
        // A null addr fails before any socket work and leaves the failure id 0.
        let id = raven_tls_connect(std::ptr::null(), std::ptr::null(), 0);
        assert_eq!(id, 0);
    }

    #[test]
    fn io_on_unknown_stream_id_reports_failure() {
        // write returns the -1 failure sentinel for an unknown stream, without
        // constructing a result String.
        assert_eq!(raven_tls_write(999_999, std::ptr::null()), -1);
        // Timeout setters are no-ops on an unknown id.
        raven_tls_set_read_timeout_ms(999_999, 1000);
        raven_tls_set_write_timeout_ms(999_999, 0);
    }
}
