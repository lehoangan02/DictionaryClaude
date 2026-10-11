//! The code that pairs a device with the sync server: where the server is,
//! the token to present, and the certificate to trust (no CA involved).

use anyhow::{anyhow, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Pairing {
    pub v: u32,
    pub name: String,
    /// host:port on the home network.
    pub lan: Option<String>,
    /// host:port reachable from anywhere.
    pub wan: Option<String>,
    pub token: String,
    pub cert: String,
}

impl Pairing {
    pub fn encode(&self) -> String {
        format!("lexis1:{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).unwrap()))
    }

    pub fn decode(code: &str) -> Result<Self> {
        let body: String = code.split_whitespace().collect();
        let body = body.strip_prefix("lexis1:").ok_or_else(|| anyhow!("that is not a LadyBird pairing code"))?;
        let p: Pairing = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(body)?).map_err(|_| anyhow!("the pairing code is damaged"))?;
        if p.lan.is_none() && p.wan.is_none() {
            return Err(anyhow!("the pairing code has no server address"));
        }
        Ok(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_junk() {
        let p = Pairing { v: 1, name: "phone".into(), lan: Some("192.168.1.7:25".into()), wan: Some("lhan.duckdns.org:5025".into()), token: "t".into(), cert: "-----BEGIN CERTIFICATE-----\nabc\n-----END CERTIFICATE-----\n".into() };
        let code = p.encode();
        assert_eq!(Pairing::decode(&code).unwrap(), p);
        // Codes survive being wrapped across lines when pasted.
        let wrapped = format!("{}\n{}", &code[..20], &code[20..]);
        assert_eq!(Pairing::decode(&wrapped).unwrap(), p);
        assert!(Pairing::decode("hello").is_err());
        assert!(Pairing::decode("lexis1:!!!").is_err());
    }
}
