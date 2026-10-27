//! What the login screen remembers between runs, and the server address check.
//!
//! The config is a two line TOML file, `%APPDATA%\AABevy\login.toml` (else `login.toml` next to the exe;
//! `AA_LOGIN_CONFIG=PATH` overrides both):
//!
//! ```toml
//! user = "name"
//! server = "127.0.0.1:1237"
//! ```
//!
//! The password is never stored.

use std::path::PathBuf;

/// The server the form starts with (`$AA_LOGIN` overrides it).
pub const DEFAULT_SERVER: &str = "127.0.0.1:1237";
/// The port used when the address has none: AAEmu's standard login port.
pub const DEFAULT_PORT: u16 = 1237;

#[derive(Clone, Debug, PartialEq)]
pub struct Saved {
    pub user: String,
    pub server: String,
}

impl Default for Saved {
    fn default() -> Self {
        Saved { user: String::new(), server: std::env::var("AA_LOGIN").unwrap_or_else(|_| DEFAULT_SERVER.into()) }
    }
}

fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("AA_LOGIN_CONFIG") {
        return Some(p.into());
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(PathBuf::from(appdata).join("AABevy").join("login.toml"));
    }
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("login.toml")))
}

/// The remembered user and server (the defaults where nothing was saved).
pub fn load() -> Saved {
    let mut saved = Saved::default();
    let Some(text) = path().and_then(|p| std::fs::read_to_string(p).ok()) else { return saved };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = unquote(value.trim());
        match key.trim() {
            "user" => saved.user = value,
            "server" if !value.is_empty() => saved.server = value,
            _ => {}
        }
    }
    saved
}

/// Remember the user and server. Failures are logged, they never stop a login.
pub fn save(saved: &Saved) {
    let Some(path) = path() else { return };
    if cfg!(target_arch = "wasm32") {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = format!("user = {}\nserver = {}\n", quote(&saved.user), quote(&saved.server));
    if let Err(e) = std::fs::write(&path, text) {
        bevy::log::warn!("login screen: cannot save {}: {e}", path.display());
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(s: &str) -> String {
    let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s);
    inner.replace("\\\"", "\"").replace("\\\\", "\\")
}

/// `host`, `host:port`, `1.2.3.4:port` or `[::1]:port` to the `HOST:PORT` the session connects to.
/// A missing port is [`DEFAULT_PORT`]; anything else wrong is the message for the screen.
pub fn parse_address(input: &str) -> Result<String, String> {
    let text = input.trim();
    if text.is_empty() {
        return Err("Enter the server address (host or IP, and port).".into());
    }
    let bad = |why: &str| Err(format!("Bad server address \"{text}\": {why}."));
    let (host, port) = if let Some(rest) = text.strip_prefix('[') {
        let Some((host, tail)) = rest.split_once(']') else { return bad("missing ]") };
        match tail {
            "" => (format!("[{host}]"), None),
            _ => match tail.strip_prefix(':') {
                Some(p) => (format!("[{host}]"), Some(p)),
                None => return bad("expected :port after ]"),
            },
        }
    } else {
        match text.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host.to_string(), Some(port)),
            Some(_) => return bad("an IPv6 address needs [brackets]"),
            None => (text.to_string(), None),
        }
    };
    let host_part = host.trim_start_matches('[').trim_end_matches(']');
    if host_part.is_empty() {
        return bad("no host");
    }
    if !host_part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':')) {
        return bad("the host has characters a host name cannot have");
    }
    let port = match port {
        None => DEFAULT_PORT,
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 => n,
            _ => return bad("the port must be a number from 1 to 65535"),
        },
    };
    Ok(format!("{host}:{port}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(parse_address("127.0.0.1:1237").unwrap(), "127.0.0.1:1237");
        assert_eq!(parse_address(" play.example.com ").unwrap(), "play.example.com:1237");
        assert_eq!(parse_address("[::1]:5").unwrap(), "[::1]:5");
        assert!(parse_address("").is_err());
        assert!(parse_address("host:abc").is_err());
        assert!(parse_address("host:0").is_err());
        assert!(parse_address("host:70000").is_err());
        assert!(parse_address(":1237").is_err());
        assert!(parse_address("a b:5").is_err());
        assert!(parse_address("::1").is_err());
    }

    #[test]
    fn quoting() {
        let s = r#"a"b\c"#;
        assert_eq!(unquote(&quote(s)), s);
    }
}
