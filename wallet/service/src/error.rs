use thiserror::Error;

/// What the wallet service can fail with. The text of each variant is what a
/// person reads, so it says what happened in plain words; the terminal wraps it
/// in its own error type (`kaspa_cli::error::Error::Service`) and says it as is.
#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Custom(String),

    // Transparent: the wallet core's own errors already read as sentences, and
    // a wrapper that adds a prefix is how "Cli error Cli error cancelled" came
    // to be printed (founder, 2026-09-18).
    #[error(transparent)]
    Wallet(#[from] kaspa_wallet_core::error::Error),

    #[error(transparent)]
    Wrpc(#[from] kaspa_wrpc_client::error::Error),

    #[error(transparent)]
    Rpc(#[from] kaspa_rpc_core::RpcError),

    #[error(transparent)]
    SerdeJson(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn custom<T: Into<String>>(msg: T) -> Self {
        Error::Custom(msg.into())
    }
}

impl From<String> for Error {
    fn from(err: String) -> Self {
        Self::Custom(err)
    }
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Self::Custom(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_reads_as_its_text() {
        assert_eq!(Error::custom("not now").to_string(), "not now");
        assert_eq!(Error::from("not now").to_string(), "not now");
        assert_eq!(Error::from(String::from("not now")).to_string(), "not now");
    }

    #[test]
    fn io_error_is_said_as_it_is() {
        let io = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        assert_eq!(Error::from(io).to_string(), "no such file");
    }
}
