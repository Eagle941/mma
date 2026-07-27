use std::env;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
pub use env_logger::WriteStyle;

pub type SharedAppConfig = Arc<dyn AppConfigProvider>;

pub trait AppConfigProvider: Send + Sync {
    fn symbol(&self) -> &str;
    fn coin(&self) -> &str;
    fn order_size(&self) -> f64;
    fn testnet(&self) -> bool;
    fn api_key(&self) -> &str;
    fn api_secret(&self) -> &str;
    fn log_filter(&self) -> &str;
    fn log_style(&self) -> WriteStyle;
}

#[derive(Clone)]
pub struct ApiCredentials {
    api_key: String,
    api_secret: String,
}
impl std::fmt::Debug for ApiCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApiCredentials")
            .field("api_key", &"<redacted>")
            .field("api_secret", &"<redacted>")
            .finish()
    }
}
impl ApiCredentials {
    pub fn new(api_key: impl Into<String>, api_secret: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_secret: api_secret.into(),
        }
    }
}

#[derive(Debug)]
pub struct AppConfig {
    symbol: String,
    coin: String,
    order_size: f64,
    testnet: bool,
    credentials: ApiCredentials,
    log_filter: String,
    log_style: WriteStyle,
}
impl AppConfig {
    /// Loads and validates application configuration from environment files and
    /// variables.
    ///
    /// # Errors
    ///
    /// Returns an error if `.env` or `.secrets` cannot be loaded, a required
    /// variable is missing or blank, or a configuration value is invalid.
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().context(".env file must be present with configuration parameters")?;
        dotenvy::from_filename(".secrets")
            .context(".secrets file must be present with API_KEY and API_SECRET")?;

        let symbol = required_env("MMA_SYMBOL")?;
        let coin = required_env("MMA_COIN")?;
        let order_size = parse_order_size(&required_env("MMA_ORDER_SIZE")?)?;
        let testnet = required_env("MMA_TESTNET")?
            .parse::<bool>()
            .context("MMA_TESTNET must be true or false")?;
        let credentials =
            ApiCredentials::new(required_env("API_KEY")?, required_env("API_SECRET")?);
        let log_filter = env::var("RUST_LOG").unwrap_or_else(|_| "warn".to_string());
        let log_style =
            parse_log_style(&env::var("RUST_LOG_STYLE").unwrap_or_else(|_| "always".to_string()))?;

        Self::new(
            symbol,
            coin,
            order_size,
            testnet,
            credentials,
            log_filter,
            log_style,
        )
    }

    /// Creates application configuration from explicit values.
    ///
    /// # Errors
    ///
    /// Returns an error if `order_size` is not finite and greater than zero.
    pub fn new(
        symbol: impl Into<String>,
        coin: impl Into<String>,
        order_size: f64,
        testnet: bool,
        credentials: ApiCredentials,
        log_filter: impl Into<String>,
        log_style: WriteStyle,
    ) -> Result<Self> {
        if !order_size.is_finite() || order_size <= 0.0 {
            bail!("MMA_ORDER_SIZE must be finite and greater than zero");
        }

        Ok(Self {
            symbol: symbol.into(),
            coin: coin.into(),
            order_size,
            testnet,
            credentials,
            log_filter: log_filter.into(),
            log_style,
        })
    }
}
impl AppConfigProvider for AppConfig {
    fn symbol(&self) -> &str {
        &self.symbol
    }

    fn coin(&self) -> &str {
        &self.coin
    }

    fn order_size(&self) -> f64 {
        self.order_size
    }

    fn testnet(&self) -> bool {
        self.testnet
    }

    fn api_key(&self) -> &str {
        &self.credentials.api_key
    }

    fn api_secret(&self) -> &str {
        &self.credentials.api_secret
    }

    fn log_filter(&self) -> &str {
        &self.log_filter
    }

    fn log_style(&self) -> WriteStyle {
        self.log_style
    }
}

fn required_env(name: &str) -> Result<String> {
    let value =
        env::var(name).with_context(|| format!("{name} environment variable must be set"))?;
    if value.trim().is_empty() {
        bail!("{name} environment variable must not be blank");
    }
    Ok(value)
}

fn parse_order_size(value: &str) -> Result<f64> {
    let order_size = value
        .parse::<f64>()
        .context("MMA_ORDER_SIZE must be a valid number")?;
    if !order_size.is_finite() || order_size <= 0.0 {
        bail!("MMA_ORDER_SIZE must be finite and greater than zero");
    }
    Ok(order_size)
}

fn parse_log_style(value: &str) -> Result<WriteStyle> {
    match value.to_ascii_lowercase().as_str() {
        "auto" => Ok(WriteStyle::Auto),
        "always" => Ok(WriteStyle::Always),
        "never" => Ok(WriteStyle::Never),
        _ => bail!("RUST_LOG_STYLE must be auto, always, or never"),
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[test]
    fn debug_redacts_credentials() {
        let config = AppConfig {
            symbol: "ADAUSDT".to_string(),
            coin: "ADA".to_string(),
            order_size: 25.0,
            testnet: true,
            credentials: ApiCredentials::new("api-key", "api-secret"),
            log_filter: "info".to_string(),
            log_style: WriteStyle::Never,
        };

        let debug_output = format!("{config:?}");

        assert!(debug_output.contains("<redacted>"));
        assert!(!debug_output.contains("api-key"));
        assert!(!debug_output.contains("api-secret"));
    }

    #[rstest]
    #[case("25", 25.0)]
    #[case("0.5", 0.5)]
    fn parse_order_size_accepts_positive_finite_values(#[case] value: &str, #[case] expected: f64) {
        let actual = parse_order_size(value).unwrap();

        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    #[rstest]
    #[case("not-a-number")]
    #[case("NaN")]
    #[case("inf")]
    #[case("0")]
    #[case("-1")]
    fn parse_order_size_rejects_invalid_values(#[case] value: &str) {
        assert!(parse_order_size(value).is_err());
    }

    #[rstest]
    #[case("auto", WriteStyle::Auto)]
    #[case("always", WriteStyle::Always)]
    #[case("never", WriteStyle::Never)]
    #[case("ALWAYS", WriteStyle::Always)]
    fn parse_log_style_accepts_supported_values(#[case] value: &str, #[case] expected: WriteStyle) {
        assert_eq!(parse_log_style(value).unwrap(), expected);
    }

    #[test]
    fn parse_log_style_rejects_invalid_value() {
        let error = parse_log_style("sometimes").unwrap_err();

        assert!(error.to_string().contains("RUST_LOG_STYLE"));
    }
}
