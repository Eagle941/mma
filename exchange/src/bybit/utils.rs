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
) -> String {
    // TODO: optimise signature generation
    let mut mac =
        HmacSha256::new_from_slice(api_secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(timestamp.as_bytes());
    mac.update(api_key.as_bytes());
    mac.update(recv_window.as_bytes());
    mac.update(params.as_bytes());

    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    hex::encode(code_bytes)
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case(
        "1658384314791",
        "test-api-key",
        "5000",
        r#"{"category":"spot","symbol":"ADAUSDT"}"#,
        "test-api-secret",
        "aa877ae98348a8b8258a38599ba916dda29f22f78e46d306a2b62e0ab51e7731"
    )]
    #[case(
        "",
        "",
        "",
        "",
        "key",
        "5d5d139563c95b5967b9bd9a8c9b233a9dedb45072794cd232dc1b74832607d0"
    )]
    #[case(
        "123",
        "api",
        "1000",
        "",
        "secret",
        "8ccc19965c13079c1ead2061f8dd32c317c342be9f3f4e9761658a05fc82456d"
    )]
    fn generate_signature_returns_expected_hmac(
        #[case] timestamp: &str,
        #[case] api_key: &str,
        #[case] recv_window: &str,
        #[case] params: &str,
        #[case] api_secret: &str,
        #[case] expected_signature: &str,
    ) {
        let signature = generate_signature(timestamp, api_key, recv_window, params, api_secret);

        assert_eq!(signature, expected_signature);
    }
}
