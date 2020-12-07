use snafu::{ResultExt, Snafu};
use std::{
    env,
    net::{IpAddr, SocketAddr},
};
use url::Url;

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub public_uri: Url,
    pub listen_address: SocketAddr,
    pub stack_overflow_client_id: String,
    pub stack_overflow_client_secret: String,
    pub stack_overflow_client_key: String,
}

impl Config {
    pub fn from_environment() -> Result<Self> {
        let database_url = env::var("DATABASE_URL").context(UnknownDatabaseUrl)?;
        let uri = env::var("WEB_PUBLIC_URI").context(UnknownWebPublicUri)?;
        let address = env::var("WEB_LISTEN_ADDRESS").context(UnknownWebListenAddress)?;
        let port = env::var("WEB_LISTEN_PORT").context(UnknownWebListenPort)?;
        let stack_overflow_client_id =
            env::var("STACK_OVERFLOW_CLIENT_ID").context(UnknownStackOverflowClientId)?;
        let stack_overflow_client_secret =
            env::var("STACK_OVERFLOW_CLIENT_SECRET").context(UnknownStackOverflowClientSecret)?;
        let stack_overflow_client_key =
            env::var("STACK_OVERFLOW_CLIENT_KEY").context(UnknownStackOverflowClientKey)?;

        let public_uri = Url::parse(&uri).context(InvalidWebPublicUri { uri })?;
        let address: IpAddr = address
            .parse()
            .context(InvalidWebListenAddress { address })?;
        let port = port.parse().context(InvalidWebListenPort { port })?;
        let listen_address = (address, port).into();

        Ok(Self {
            database_url,
            public_uri,
            listen_address,
            stack_overflow_client_id,
            stack_overflow_client_secret,
            stack_overflow_client_key,
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

    #[snafu(display("STACK_OVERFLOW_CLIENT_ID must be set"))]
    UnknownStackOverflowClientId { source: env::VarError },

    #[snafu(display("STACK_OVERFLOW_CLIENT_SECRET must be set"))]
    UnknownStackOverflowClientSecret { source: env::VarError },

    #[snafu(display("STACK_OVERFLOW_CLIENT_KEY must be set"))]
    UnknownStackOverflowClientKey { source: env::VarError },
}

type Result<T, E = Error> = std::result::Result<T, E>;
