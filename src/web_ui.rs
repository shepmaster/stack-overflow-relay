use crate::{GlobalConfig, GlobalStackOverflowConfig};
use http::{header, StatusCode};
use snafu::Snafu;
use std::convert::Infallible;
use tracing::error;
use warp::{reply, Filter, Rejection, Reply};

pub(crate) async fn serve(config: GlobalConfig, so_config: GlobalStackOverflowConfig) {
    let oauth = oauth::routes(config, so_config);

    let root = warp::path::end().map(|| warp::reply::html(html::root().into_string()));

    let routes = oauth.or(root);
    let routes = routes.recover(report_invalid);

    warp::serve(routes).run(config.listen_address).await
}

fn redirect_to(location: impl AsRef<str>) -> impl Reply {
    let r = reply::reply();
    let r = reply::with_header(r, header::LOCATION, location.as_ref());
    let r = reply::with_status(r, StatusCode::TEMPORARY_REDIRECT);

    r
}

async fn report_invalid(r: Rejection) -> Result<impl Reply, Infallible> {
    let internal = || {
        Ok(warp::reply::with_status(
            String::from("An internal error occurred"),
            StatusCode::INTERNAL_SERVER_ERROR,
        ))
    };

    if let Some(e) = r.find::<Error>() {
        use Error::*;
        match e {
            StateParameterMismatch { .. } => Ok(warp::reply::with_status(
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )),
            UnableToGetOauthEntryUrl { .. }
            | UnableToGetOauthAccessToken { .. }
            | UnableToBuildRedirectUri { .. } => {
                error!("Unhandled web UI error: {}", e);
                internal()
            }
        }
    } else {
        error!("Unhandled web UI error: {:?}", r);
        internal()
    }
}

#[derive(Debug, Snafu)]
enum Error {
    StateParameterMismatch,

    UnableToGetOauthEntryUrl {
        source: crate::stack_overflow::Error,
    },

    UnableToGetOauthAccessToken {
        source: crate::stack_overflow::Error,
    },

    UnableToBuildRedirectUri {
        source: url::ParseError,
    },
}

type Result<T, E = Error> = std::result::Result<T, E>;

impl warp::reject::Reject for Error {}

impl From<Error> for warp::Rejection {
    fn from(other: Error) -> Self {
        warp::reject::custom(other)
    }
}

mod oauth {
    use super::{
        redirect_to, Result, StateParameterMismatch, UnableToBuildRedirectUri,
        UnableToGetOauthAccessToken, UnableToGetOauthEntryUrl,
    };
    use crate::{GlobalConfig, GlobalStackOverflowConfig};
    use once_cell::sync::Lazy;
    use parking_lot::Mutex;
    use rand::{distributions::Alphanumeric, Rng, SeedableRng};
    use serde::Deserialize;
    use snafu::{ensure, ResultExt};
    use std::mem;
    use url::Url;
    use warp::{
        filters::{query, BoxedFilter},
        Filter, Rejection,
    };

    // This is not appropriate for multiple concurrent users
    static OAUTH_STATE: Lazy<Mutex<String>> = Lazy::new(Default::default);

    pub(crate) fn routes(
        config: GlobalConfig,
        so_config: GlobalStackOverflowConfig,
    ) -> BoxedFilter<(impl warp::Reply,)> {
        warp::path!("oauth" / "stackoverflow" / ..)
            .and(begin(config, so_config).or(complete(config)))
            .boxed()
    }

    fn begin(
        config: GlobalConfig,
        so_config: GlobalStackOverflowConfig,
    ) -> BoxedFilter<(impl warp::Reply,)> {
        warp::path("begin")
            .and_then(move || async move {
                let rng = rand::rngs::StdRng::from_entropy();
                let state: String = rng.sample_iter(&Alphanumeric).take(64).collect();

                *OAUTH_STATE.lock() = state.clone();

                let redirect_uri = redirect_uri(config)?.to_string();

                let u = so_config
                    .oauth_entry_url(&redirect_uri, &state)
                    .context(UnableToGetOauthEntryUrl)?;

                Ok::<_, Rejection>(redirect_to(u.to_string()))
            })
            .boxed()
    }

    #[derive(Debug, Deserialize)]
    struct CompleteParams {
        code: String,
        state: String,
    }

    fn complete(config: GlobalConfig) -> BoxedFilter<(impl warp::Reply,)> {
        warp::path("complete")
            .and(query::query())
            .and_then(move |params: CompleteParams| {
                async move {
                    let expected_state = mem::take(&mut *OAUTH_STATE.lock());

                    ensure!(params.state == expected_state, StateParameterMismatch);

                    let redirect_uri = redirect_uri(config)?.to_string();

                    let req = crate::stack_overflow::AccessTokenRequest {
                        client_id: &*config.stack_overflow_client_id,
                        client_secret: &*config.stack_overflow_client_secret,
                        code: &*params.code,
                        redirect_uri: &*redirect_uri,
                    };

                    let resp = crate::stack_overflow::get_access_token(&req)
                        .await
                        .context(UnableToGetOauthAccessToken)?;

                    dbg!(&resp);
                    // what do we persist?

                    Ok::<_, warp::Rejection>(redirect_to(config.public_uri.to_string()))
                }
            })
            .boxed()
    }

    fn redirect_uri(config: &crate::Config) -> Result<Url> {
        config
            .public_uri
            .join("oauth/stackoverflow/complete")
            .context(UnableToBuildRedirectUri)
    }
}

mod html {
    use maud::{html, Markup};

    pub fn root() -> Markup {
        page(|| {
            html! {
                a href="/oauth/stackoverflow/begin" { "Start login" }
            }
        })
    }

    fn page(body: impl FnOnce() -> Markup) -> Markup {
        html! {
            (maud::DOCTYPE)
                html {
                    head {
                        title { "Stack Overflow Relay" }
                    }
                    body {
                        (body())
                    }
                }
        }
    }
}
