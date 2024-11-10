use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use actix_web::{
    web,
    dev::Payload,
    Error,
    error::ResponseError,
    FromRequest,
    HttpRequest,
    HttpResponse,
    http::{header, StatusCode},
};

use std::fmt::{Debug, Display};
use std::future::{ready, Ready};
use serde::{Deserialize, Serialize};
use crate::appstate::AppState;

// Custom error type that implements ResponseError
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

        let credentials = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|s| s.to_str().ok()?.strip_prefix("Basic "))
            .and_then(|s| BASE64.decode(s).ok())
            .and_then(|vec| String::from_utf8(vec).ok());

        let data = req.app_data::<web::Data<AppState>>().unwrap(); // Get data from App State
        let config = &data.config;

        match credentials {
            Some(credentials_string) => {
                // at this point, we have the credentials in the form of "username:password"
                // and we could do some proper OAuth2 validation.
                if config.authentication.credentials.contains(&credentials_string) {
                    ready(Ok(Authorized {
                        credentials: credentials_string,
                    }))
                } else {
                    let error = UnauthorizedError { message: "Unauthorized" };
                    ready(Err(error.into()))
                }
            }
            None => {
                let error = UnauthorizedError { message: "Unauthorized" };
                ready(Err(error.into()))
            }
        }
    }
}
