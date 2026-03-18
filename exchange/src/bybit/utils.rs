use std::env;

use hex;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn generate_signature(
    timestamp: &str,
    api_key: &str,
    recv_window: &str,
    params: &str,
    api_secret: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // TODO: optimise signature generation
    let mut mac =
        HmacSha256::new_from_slice(api_secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(timestamp.as_bytes());
    mac.update(api_key.as_bytes());
    mac.update(recv_window.as_bytes());
    mac.update(params.as_bytes());

    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    Ok(hex::encode(code_bytes))
}

pub fn is_testnet() -> bool {
    env::var("MMA_TESTNET")
        .expect("MMA_TESTNET env variable must not be blank.")
        .parse()
        .unwrap()
}

pub fn get_base_url() -> String {
    if is_testnet() {
        return "https://api-testnet.bybit.com".to_string();
    }
    "https://api.bybit.com".to_string()
}

// TODO: add tests for `generate_signature`
