use crate::{domain::AccountId, GlobalConfig, GlobalStackOverflowConfig};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use snafu::{ensure, OptionExt, Snafu};
use std::{
    collections::BTreeMap,
    convert::{Infallible, TryInto},
};
use tracing::{error, info};
use warp::{
    filters::cookie,
    http::{header, StatusCode},
    path, reply, Filter, Rejection, Reply,
};

#[derive(Debug, Clone, PartialOrd, Ord, PartialEq, Eq)]
struct SessionId([u8; 32]);

impl SessionId {
    fn from_cookie(s: &str) -> Option<Self> {
        let bytes = hex::decode(s).ok()?;
        let bytes = bytes.try_into().ok()?;
        Some(Self(bytes))
    }

    fn to_cookie(&self) -> String {
        hex::encode(&self.0)
    }
}

#[derive(Debug, Clone, Default)]
struct SessionData {
    oauth_state: Option<String>,
    account_id: Option<AccountId>,
}

#[derive(Debug, Clone)]
struct Session(SessionId, SessionData);

impl Session {
    fn set_oauth_state(&mut self, oauth_state: impl Into<String>) {
        self.1.oauth_state = Some(oauth_state.into())
    }

    fn take_oauth_state(&mut self) -> Option<String> {
        self.1.oauth_state.take()
    }

    fn set_account_id(&mut self, account_id: AccountId) {
        self.1.account_id = Some(account_id);
    }
}

#[derive(Debug, Default)]
struct Sessions(BTreeMap<SessionId, SessionData>);

impl Sessions {
    fn create(&mut self) -> SessionId {
        use rand::{Rng, SeedableRng};

        let mut rng = rand::rngs::StdRng::from_entropy();
        let mut id;
        loop {
            id = SessionId(rng.gen());
            if !self.0.contains_key(&id) {
                break;
            }
        }

        let session = SessionData::default();
        self.0.insert(id.clone(), session);

        id
    }

    fn for_id(&self, id: &str) -> Option<Session> {
        let id = SessionId::from_cookie(id)?;
        let data = self.0.get(&id).cloned()?;
        Some(Session(id, data))
    }

    fn save(&mut self, session: Session) {
        self.0.insert(session.0, session.1);
    }
}

static SESSIONS: Lazy<Mutex<Sessions>> = Lazy::new(Default::default);

pub(crate) async fn serve(
    config: GlobalConfig,
    so_config: GlobalStackOverflowConfig,
    register_flow: crate::flow::RegisterFlow,
) {
    let oauth = oauth::routes(config, so_config, register_flow);

    let auth_root = path::end()
        .and(auth_session())
        .map(|session| format!("{:?}", session));
    let unauth_root = path::end().map(|| {
        let id = SESSIONS.lock().create();
        let h = warp::reply::html(html::root().into_string());
        reply::with_header(
            h,
            header::SET_COOKIE,
            format!("id={}; Secure; HttpOnly;", id.to_cookie()),
        ) // samesite?
    });
    let root = auth_root.or(unauth_root);

    let routes = oauth.or(root);
    let routes = routes.recover(report_invalid);

    info!("Starting web server at {}", &config.listen_address);
    warp::serve(routes).run(config.listen_address).await
}

fn session() -> warp::filters::BoxedFilter<(Session,)> {
    cookie::cookie("id")
        .and_then(|id: String| async move {
            let sessions = SESSIONS.lock();
            sessions
                .for_id(&id)
                .context(NotAuthenticated)
                .map_err(Rejection::from)
        })
        .boxed()
}

fn auth_session() -> warp::filters::BoxedFilter<(Session,)> {
    session()
        .and_then(|session: Session| async move {
            ensure!(session.1.account_id.is_some(), NotAuthenticated);
            Ok::<_, Rejection>(session)
        })
        .boxed()
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
            NotAuthenticated => Ok(warp::reply::with_status(
                "Not authorized".to_string(),
                StatusCode::UNAUTHORIZED,
            )),
            StateParameterMismatch { .. } => Ok(warp::reply::with_status(
                e.to_string(),
                StatusCode::BAD_REQUEST,
            )),
            UnableToGetOauthEntryUrl { .. }
            | UnableToCompleteRegistration { .. }
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
    NotAuthenticated,

    StateParameterMismatch,

    UnableToCompleteRegistration {
        source: crate::flow::Error,
    },

    UnableToGetOauthEntryUrl {
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
        redirect_to, session, Result, Session, StateParameterMismatch, UnableToBuildRedirectUri,
        UnableToCompleteRegistration, UnableToGetOauthEntryUrl, SESSIONS,
    };
    use crate::{GlobalConfig, GlobalStackOverflowConfig};
    use rand::{distributions::Alphanumeric, Rng, SeedableRng};
    use serde::Deserialize;
    use snafu::{ensure, ResultExt};
    use url::Url;
    use warp::{
        filters::{query, BoxedFilter},
        Filter, Rejection,
    };

    pub(crate) fn routes(
        config: GlobalConfig,
        so_config: GlobalStackOverflowConfig,
        register_flow: crate::flow::RegisterFlow,
    ) -> BoxedFilter<(impl warp::Reply,)> {
        warp::path!("oauth" / "stackoverflow" / ..)
            .and(begin(config, so_config).or(complete(config, register_flow)))
            .boxed()
    }

    fn begin(
        config: GlobalConfig,
        so_config: GlobalStackOverflowConfig,
    ) -> BoxedFilter<(impl warp::Reply,)> {
        warp::path("begin")
            .and(session())
            .and_then(move |mut session: Session| async move {
                let rng = rand::rngs::StdRng::from_entropy();
                let state: String = rng.sample_iter(&Alphanumeric).take(64).collect();

                session.set_oauth_state(state.clone());
                SESSIONS.lock().save(session);

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

    fn complete(
        config: GlobalConfig,
        flow: crate::flow::RegisterFlow,
    ) -> BoxedFilter<(impl warp::Reply,)> {
        warp::path("complete")
            .and(session())
            .and(query::query())
            .and_then(move |mut session: Session, params: CompleteParams| {
                let mut flow = flow.clone();
                async move {
                    let expected_state = session.take_oauth_state();
                    SESSIONS.lock().save(session.clone());

                    ensure!(
                        expected_state.map_or(false, |e| params.state == e),
                        StateParameterMismatch
                    );

                    let redirect_uri = redirect_uri(config)?.to_string();

                    let account_id = flow
                        .register(&params.code, &redirect_uri)
                        .await
                        .context(UnableToCompleteRegistration)?;

                    session.set_account_id(account_id);
                    SESSIONS.lock().save(session);

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
