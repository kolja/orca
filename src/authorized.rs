use actix_web::{
    dev::Payload,
    error::ResponseError,
    http::{header, header::HeaderValue, StatusCode},
    web, Error, FromRequest, HttpRequest, HttpResponse,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use crate::hash;
use crate::authdoc;
use crate::config::Config;
use crate::appstate::AppState;
use crate::public;
use crate::routes::origin;
use serde_derive::{Deserialize, Serialize};
use std::fmt::{Debug, Display};
use std::future::{ready, Ready};

#[derive(Debug)]
struct UnauthorizedError {
    document: String,
}

impl Display for UnauthorizedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.document)
    }
}

impl ResponseError for UnauthorizedError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code())
            .append_header(("WWW-Authenticate", r#"Basic realm="Login Required""#))
            .content_type(authdoc::AUTHENTICATION)
            .body(self.to_string())
    }

    fn status_code(&self) -> StatusCode {
        StatusCode::UNAUTHORIZED
    }
}

pub const GUEST: &str = "Guest";

#[derive(Serialize, Deserialize)]
pub struct Authorized {
    pub login: String,
    limited: bool, // a request sees only what is listed in `public`
}

impl Authorized {
    /// The catalog uses this to list the libraries a client can actually open
    pub fn may_reach(&self, config: &Config, path: &str) -> bool {
        !self.limited || public::allows(config.public(), path)
    }
}

fn verify_credentials(header: &HeaderValue, config: &Config) -> Option<Authorized> {
    let credentials = header.to_str().ok()?;
    credentials.strip_prefix("Basic ")
               .and_then(|s| BASE64.decode(s).ok())
               .and_then(|vec| String::from_utf8(vec).ok())
               .and_then(|loginpassword| {
                   let (login, password) = loginpassword.split_once(":")?;
                   let hash = config.authentication.as_ref()?.login.get(login)?;
                   match hash::verify_password(password, hash).ok()? {
                       true => Some(Authorized {
                            login: login.to_string(),
                            limited: false,
                       }),
                       false => None,
                   }
               })
}

impl FromRequest for Authorized {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let data = req.app_data::<web::Data<AppState>>().unwrap();
        let config = &data.config;
        let path = req.uri().path();

        // `[authentication]` section: everything is public
        if config.authentication.is_none() {
            return ready(Ok(Authorized { login: GUEST.to_string(), limited: false }));
        }

        let result = req.headers().get(header::AUTHORIZATION)
                                  .and_then(|header| verify_credentials(header, config));

        match result {
            Some(auth) => ready(Ok(auth)),
            None if public::allows(config.public(), path) => ready(Ok(Authorized {
                login: GUEST.to_string(),
                limited: true,
            })),
            None => {
                let error = UnauthorizedError {
                    document: authdoc::body(&origin(req, config)),
                };
                ready(Err(error.into()))
            }
        }
    }
}
