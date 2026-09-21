//! Wallet-connect URI. QR rendering is vendored `qr-code-styling` in the page.

/// Fully Noded / StandUp URI that Sparrow-class wallets can scan.
pub fn standup_uri(user: &str, pass: &str, host: &str, port: u16, tls: bool) -> String {
    let user = pct(user);
    let pass = pct(pass);
    let tls_q = if tls { "&tls=true" } else { "" };
    format!("btcstandup://{user}:{pass}@{host}:{port}/?label=Commons{tls_q}")
}

fn pct(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_password_specials() {
        let u = standup_uri("user", "p@ss", "127.0.0.1", 48332, false);
        assert_eq!(u, "btcstandup://user:p%40ss@127.0.0.1:48332/?label=Commons");
    }

    #[test]
    fn tls_flag_appended() {
        let u = standup_uri("btc", "x", "10.0.0.2", 8332, true);
        assert!(u.ends_with("&tls=true"));
    }
}
