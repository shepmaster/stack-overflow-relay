use snafu::{ResultExt, Snafu};
use std::{
    env,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use url::Url;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub public_uri: Url,
    pub listen_address: SocketAddr,
    pub caffeine_interval: Option<Duration>,
}

impl Config {
    pub fn from_environment() -> Result<Self> {
        let database_url = env::var("DATABASE_URL").context(UnknownDatabaseUrlSnafu)?;
        let uri = env::var("WEB_PUBLIC_URI").context(UnknownWebPublicUriSnafu)?;
        let address = env::var("WEB_LISTEN_ADDRESS").context(UnknownWebListenAddressSnafu)?;
        let port = env::var("WEB_LISTEN_PORT").or_else(|_| env::var("PORT"));
        let port = port.context(UnknownWebListenPortSnafu)?;
        let caffeine_interval = env::var("PREVENT_HEROKU_SLEEP").ok();

        let public_uri = Url::parse(&uri).context(InvalidWebPublicUriSnafu { uri })?;
        let address: IpAddr = address
            .parse()
            .context(InvalidWebListenAddressSnafu { address })?;
        let port = port.parse().context(InvalidWebListenPortSnafu { port })?;
        let listen_address = (address, port).into();
        let caffeine_interval = caffeine_interval
            .and_then(|i| i.parse().ok())
            .map(Duration::from_secs);

        Ok(Self {
            database_url,
            public_uri,
            listen_address,
            caffeine_interval,
        })
    }
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("DATABASE_URL must be set"))]
    UnknownDatabaseUrl { source: env::VarError },

    #[snafu(display("WEB_LISTEN_ADDRESS must be set"))]
    UnknownWebListenAddress { source: env::VarError },

    #[snafu(display("WEB_LISTEN_ADDRESS is invalid"))]
    InvalidWebListenAddress {
        source: std::net::AddrParseError,
        address: String,
    },

    #[snafu(display("WEB_LISTEN_PORT must be set"))]
    UnknownWebListenPort { source: env::VarError },

    #[snafu(display("WEB_LISTEN_PORT is invalid"))]
    InvalidWebListenPort {
        source: std::num::ParseIntError,
        port: String,
    },

    #[snafu(display("WEB_PUBLIC_URI must be set"))]
    UnknownWebPublicUri { source: env::VarError },

    #[snafu(display("WEB_PUBLIC_URI is invalid"))]
    InvalidWebPublicUri {
        source: url::ParseError,
        uri: String,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;
