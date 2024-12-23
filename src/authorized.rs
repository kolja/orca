use actix_web::{
    dev::Payload,
    error::ResponseError,
    http::{header, StatusCode},
    web, Error, FromRequest, HttpRequest, HttpResponse,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use crate::appstate::AppState;
use crate::hash::LoginData;
use serde_derive::{Deserialize, Serialize};
use std::fmt::{Debug, Display};
use std::future::{ready, Ready};

#[derive(Debug)]
struct UnauthorizedError {
    message: &'static str,
}

impl Display for UnauthorizedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl ResponseError for UnauthorizedError {
    fn error_response(&self) -> HttpResponse {
        HttpResponse::build(self.status_code())
            .append_header(("WWW-Authenticate", r#"Basic realm="Login Required""#))
            .body(self.to_string())
    }

    fn status_code(&self) -> StatusCode {
        StatusCode::UNAUTHORIZED
    }
}

#[derive(Serialize, Deserialize)]
pub struct Authorized {
    pub credentials: String,
}

impl FromRequest for Authorized {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let data = req.app_data::<web::Data<AppState>>().unwrap();
        let config = &data.config;

    let result = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.strip_prefix("Basic "))
        .and_then(|s| BASE64.decode(s).ok())
        .and_then(|vec| String::from_utf8(vec).ok())
        .and_then(|credentials_string| {
            let (login, password) = credentials_string.split_once(":")?;

            if let Some(hash) = config.authentication.get(login) {
                let login_data = LoginData::new_with_hash(login, password, hash).ok()?;
                if login_data.verify_password().ok()? {
                    return Some(Authorized {
                        credentials: credentials_string,
                    });
                }
            }

            None
        });

        match result {
            Some(auth) => ready(Ok(auth)),
            None => {
                let error = UnauthorizedError {
                    message: "Unauthorized",
                };
                ready(Err(error.into()))
            }
        }
    }
}
