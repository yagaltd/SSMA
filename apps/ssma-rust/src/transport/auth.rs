use crate::gateway::{api_error, cookie_value, ApiResult, AppState};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::PasswordHasher;
use argon2::password_hash::PasswordVerifier;
use argon2::password_hash::SaltString;
use argon2::Argon2;
use argon2::PasswordHash;
use axum::extract::State;
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use oauth2::basic::BasicClient;
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

// --- Types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserRecord {
    pub(crate) id: String,
    pub(crate) email: String,
    pub(crate) password_hash: String,
    pub(crate) name: String,
    pub(crate) role: String,
    pub(crate) status: String,
    #[serde(default = "default_email_verified")]
    pub(crate) email_verified: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) last_login_at: Option<i64>,
    #[serde(default)]
    pub(crate) refresh_token_hash: Option<String>,
    #[serde(default)]
    pub(crate) refresh_token_expires_at: Option<i64>,
    #[serde(default)]
    pub(crate) verification_token_hash: Option<String>,
    #[serde(default)]
    pub(crate) verification_token_expires_at: Option<i64>,
    #[serde(default)]
    pub(crate) reset_token_hash: Option<String>,
    #[serde(default)]
    pub(crate) reset_token_expires_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PersistedUsers {
    version: u8,
    users: Vec<UserRecord>,
}

fn default_email_verified() -> bool {
    true
}

pub(crate) struct UserStore {
    path: PathBuf,
    state: Arc<Mutex<PersistedUsers>>,
}

impl UserStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let persisted = if path.exists() {
            let data = fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str::<PersistedUsers>(&data).unwrap_or(PersistedUsers {
                version: 1,
                users: Vec::new(),
            })
        } else {
            PersistedUsers {
                version: 1,
                users: Vec::new(),
            }
        };
        Self {
            path,
            state: Arc::new(Mutex::new(persisted)),
        }
    }

    fn persist(&self) -> Result<(), String> {
        let data = self.state.lock().map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(&*data).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&tmp, &self.path).map_err(|e| e.to_string())?;
        // Sync parent directory for crash durability
        if let Some(parent) = self.path.parent() {
            if let Ok(dir) = fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    }

    pub(crate) fn find_by_email(&self, email: &str) -> Option<UserRecord> {
        let data = self.state.lock().ok()?;
        data.users.iter().find(|u| u.email == email).cloned()
    }

    pub(crate) fn find_by_id(&self, id: &str) -> Option<UserRecord> {
        let data = self.state.lock().ok()?;
        data.users.iter().find(|u| u.id == id).cloned()
    }

    pub(crate) fn create(&self, record: UserRecord) -> Result<UserRecord, String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            if data.users.iter().any(|u| u.email == record.email) {
                return Err("EMAIL_TAKEN".to_string());
            }
            data.users.push(record.clone());
        }
        self.persist()?;
        Ok(record)
    }

    pub(crate) fn update_login(&self, id: &str) -> Result<(), String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| u.id == id) {
                user.last_login_at = Some(now);
                user.updated_at = now;
            }
        }
        self.persist()
    }

    pub(crate) fn set_refresh_token(
        &self,
        id: &str,
        token_hash: String,
        expires_at: i64,
    ) -> Result<(), String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| u.id == id) {
                user.refresh_token_hash = Some(token_hash);
                user.refresh_token_expires_at = Some(expires_at);
                user.updated_at = now;
            }
        }
        self.persist()
    }

    pub(crate) fn clear_refresh_token(&self, id: &str) -> Result<(), String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| u.id == id) {
                user.refresh_token_hash = None;
                user.refresh_token_expires_at = None;
                user.updated_at = now;
            }
        }
        self.persist()
    }

    pub(crate) fn find_by_refresh_token_hash(&self, token_hash: &str) -> Option<UserRecord> {
        let now = crate::runtime::now_millis();
        let data = self.state.lock().ok()?;
        data.users
            .iter()
            .find(|u| {
                u.refresh_token_hash.as_deref() == Some(token_hash)
                    && u.refresh_token_expires_at.unwrap_or_default() > now
            })
            .cloned()
    }

    pub(crate) fn set_verification_token(
        &self,
        id: &str,
        token_hash: String,
        expires_at: i64,
    ) -> Result<(), String> {
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| u.id == id) {
                user.email_verified = false;
                user.verification_token_hash = Some(token_hash);
                user.verification_token_expires_at = Some(expires_at);
                user.updated_at = now;
            }
        }
        self.persist()
    }

    pub(crate) fn mark_email_verified_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<UserRecord>, String> {
        let mut found = None;
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| {
                u.verification_token_hash.as_deref() == Some(token_hash)
                    && u.verification_token_expires_at.unwrap_or_default() > now
            }) {
                user.email_verified = true;
                if user.status == "pending_verification" {
                    user.status = "active".to_string();
                }
                user.verification_token_hash = None;
                user.verification_token_expires_at = None;
                user.updated_at = now;
                found = Some(user.clone());
            }
        }
        self.persist()?;
        Ok(found)
    }

    pub(crate) fn issue_reset_token_by_email(
        &self,
        email: &str,
        token_hash: String,
        expires_at: i64,
    ) -> Result<Option<UserRecord>, String> {
        let mut found = None;
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| u.email == email) {
                user.reset_token_hash = Some(token_hash);
                user.reset_token_expires_at = Some(expires_at);
                user.updated_at = now;
                found = Some(user.clone());
            }
        }
        self.persist()?;
        Ok(found)
    }

    pub(crate) fn reset_password_by_token_hash(
        &self,
        token_hash: &str,
        next_password_hash: String,
    ) -> Result<Option<UserRecord>, String> {
        let mut found = None;
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data.users.iter_mut().find(|u| {
                u.reset_token_hash.as_deref() == Some(token_hash)
                    && u.reset_token_expires_at.unwrap_or_default() > now
            }) {
                user.password_hash = next_password_hash;
                user.reset_token_hash = None;
                user.reset_token_expires_at = None;
                user.refresh_token_hash = None;
                user.refresh_token_expires_at = None;
                user.updated_at = now;
                found = Some(user.clone());
            }
        }
        self.persist()?;
        Ok(found)
    }

    pub(crate) fn issue_verification_token_by_email(
        &self,
        email: &str,
        token_hash: String,
        expires_at: i64,
    ) -> Result<Option<UserRecord>, String> {
        let mut found = None;
        {
            let mut data = self.state.lock().map_err(|e| e.to_string())?;
            let now = crate::runtime::now_millis();
            if let Some(user) = data
                .users
                .iter_mut()
                .find(|u| u.email == email && !u.email_verified)
            {
                user.verification_token_hash = Some(token_hash);
                user.verification_token_expires_at = Some(expires_at);
                user.updated_at = now;
                found = Some(user.clone());
            }
        }
        self.persist()?;
        Ok(found)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TokenClaims {
    sub: String,
    role: String,
    iss: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegisterRequest {
    email: String,
    password: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OidcCallbackQuery {
    code: String,
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForgotPasswordRequest {
    email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResetPasswordRequest {
    token: String,
    new_password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VerifyEmailRequest {
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResendVerificationRequest {
    email: String,
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| e.to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    let parsed = match PasswordHash::new(hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn issue_jwt(user: &UserRecord, state: &AppState) -> Result<String, String> {
    let now = crate::runtime::now_secs();
    let claims = TokenClaims {
        sub: user.id.clone(),
        role: user.role.clone(),
        iss: state.config.jwt_issuer.clone(),
        aud: state.config.jwt_audience.clone(),
        iat: now,
        exp: now + state.config.access_ttl_ms / 1000,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(state.config.auth_jwt_secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

fn auth_cookie_value(jwt: &str, config: &crate::config::Config) -> String {
    let secure = if config.auth_cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let same_site = format!("; SameSite={}", config.auth_cookie_same_site);
    format!(
        "{}={}; Path=/; HttpOnly{}{}",
        config.auth_cookie_name, jwt, same_site, secure
    )
}

fn refresh_cookie_value(token: &str, config: &crate::config::Config) -> String {
    let secure = if config.auth_cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let same_site = format!("; SameSite={}", config.auth_cookie_same_site);
    format!(
        "{}={}; Path=/; HttpOnly{}{}",
        config.refresh_cookie_name, token, same_site, secure
    )
}

fn clear_cookie_value(config: &crate::config::Config) -> String {
    let secure = if config.auth_cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let same_site = format!("; SameSite={}", config.auth_cookie_same_site);
    format!(
        "{}=; Path=/; HttpOnly{}{}; Max-Age=0",
        config.auth_cookie_name, same_site, secure
    )
}

fn clear_refresh_cookie_value(config: &crate::config::Config) -> String {
    let secure = if config.auth_cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let same_site = format!("; SameSite={}", config.auth_cookie_same_site);
    format!(
        "{}=; Path=/; HttpOnly{}{}; Max-Age=0",
        config.refresh_cookie_name, same_site, secure
    )
}

fn hash_token(secret: &str, token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn random_token() -> String {
    format!("{}.{}", Uuid::new_v4(), Uuid::new_v4())
}

fn set_request_id_header(headers: &mut HeaderMap, request_id: &str) {
    if let Ok(value) = HeaderValue::from_str(request_id) {
        headers.insert("x-request-id", value);
    }
}

fn issue_refresh_cookie_for_user(
    state: &Arc<AppState>,
    user: &UserRecord,
    headers: &mut HeaderMap,
) -> Result<(), String> {
    if !state.config.auth_refresh_enabled {
        return Ok(());
    }
    let refresh_token = random_token();
    let refresh_hash = hash_token(&state.config.auth_jwt_secret, &refresh_token);
    let expires_at = crate::runtime::now_millis() + state.config.refresh_ttl_ms as i64;
    state
        .user_store
        .set_refresh_token(&user.id, refresh_hash, expires_at)?;
    if let Ok(val) = HeaderValue::from_str(&refresh_cookie_value(&refresh_token, &state.config)) {
        headers.append(SET_COOKIE, val);
    }
    Ok(())
}

fn user_to_json(user: &UserRecord) -> Value {
    json!({
        "id": user.id,
        "email": user.email,
        "name": user.name,
        "role": user.role,
        "status": user.status,
        "emailVerified": user.email_verified,
        "createdAt": user.created_at,
        "updatedAt": user.updated_at,
        "lastLoginAt": user.last_login_at,
    })
}

/// Wrap user in envelope expected by CSMA frontend: { status, user }
fn user_response(user: &UserRecord) -> Value {
    json!({
        "status": "ok",
        "user": user_to_json(user),
    })
}

// --- Handlers ---

pub(crate) async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<RegisterRequest>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if body.email.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "EMAIL_REQUIRED"));
    }
    if body.password.len() < 8 {
        return Err(api_error(StatusCode::BAD_REQUEST, "PASSWORD_TOO_SHORT"));
    }
    if body.name.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "NAME_REQUIRED"));
    }

    let now = crate::runtime::now_millis();
    let require_verification = state.config.auth_require_email_verification;
    let user = UserRecord {
        id: Uuid::new_v4().to_string(),
        email: body.email.trim().to_lowercase(),
        password_hash: hash_password(&body.password)
            .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "HASH_FAILED"))?,
        name: body.name,
        role: "user".to_string(),
        status: if require_verification {
            "pending_verification".to_string()
        } else {
            "active".to_string()
        },
        email_verified: !require_verification,
        created_at: now,
        updated_at: now,
        last_login_at: None,
        refresh_token_hash: None,
        refresh_token_expires_at: None,
        verification_token_hash: None,
        verification_token_expires_at: None,
        reset_token_hash: None,
        reset_token_expires_at: None,
    };

    let saved = state.user_store.create(user).map_err(|e| {
        if e == "EMAIL_TAKEN" {
            api_error(StatusCode::CONFLICT, "EMAIL_TAKEN")
        } else {
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "CREATE_FAILED")
        }
    })?;

    let jwt = issue_jwt(&saved, &state)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "JWT_FAILED"))?;

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&auth_cookie_value(&jwt, &state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    let _ = issue_refresh_cookie_for_user(&state, &saved, &mut response_headers);
    set_request_id_header(&mut response_headers, &request_id);

    if require_verification {
        let token = random_token();
        let token_hash = hash_token(&state.config.auth_jwt_secret, &token);
        let expires_at = crate::runtime::now_millis() + state.config.email_verify_ttl_ms as i64;
        let _ = state
            .user_store
            .set_verification_token(&saved.id, token_hash, expires_at);
        let _ = state
            .backend
            .auth_outbox_event(
                "verify_email",
                &saved.email,
                json!({
                    "token": token,
                    "expiresAt": expires_at,
                    "userId": saved.id,
                    "name": saved.name
                }),
                &crate::adapters::backend::BackendContext {
                    site: "default".to_string(),
                    actor_key: Some(format!("user:{}", saved.id)),
                    connection_id: None,
                    ip: Some(crate::gateway::connection_ip_from_headers(&headers)),
                    user_agent: headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    user: Some(crate::adapters::backend::BackendUser {
                        id: Some(saved.id.clone()),
                        role: saved.role.clone(),
                    }),
                },
                Some(&request_id),
            )
            .await;
    }

    Ok((
        StatusCode::CREATED,
        response_headers,
        Json(user_response(&saved)),
    ))
}

pub(crate) async fn login(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    let user = state
        .user_store
        .find_by_email(&body.email.trim().to_lowercase())
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"))?;

    if !verify_password(&body.password, &user.password_hash) {
        return Err(api_error(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS"));
    }
    if state.config.auth_require_email_verification && !user.email_verified {
        return Err(api_error(StatusCode::FORBIDDEN, "EMAIL_NOT_VERIFIED"));
    }

    let _ = state.user_store.update_login(&user.id);

    let jwt = issue_jwt(&user, &state)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "JWT_FAILED"))?;

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&auth_cookie_value(&jwt, &state.config)) {
        response_headers.append(SET_COOKIE, val);
    }

    // Re-fetch to get updated last_login_at
    let updated = state.user_store.find_by_id(&user.id).unwrap_or(user);
    let _ = issue_refresh_cookie_for_user(&state, &updated, &mut response_headers);
    set_request_id_header(&mut response_headers, &request_id);

    Ok((
        StatusCode::OK,
        response_headers,
        Json(user_response(&updated)),
    ))
}

pub(crate) async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if let Some(token) = cookie_value(&headers, &state.config.auth_cookie_name) {
        let mut validation = jsonwebtoken::Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        validation.validate_aud = false;
        if let Ok(decoded) = jsonwebtoken::decode::<crate::gateway::AuthClaims>(
            &token,
            &jsonwebtoken::DecodingKey::from_secret(state.config.auth_jwt_secret.as_bytes()),
            &validation,
        ) {
            let _ = state.user_store.clear_refresh_token(&decoded.claims.sub);
        }
    }
    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&clear_cookie_value(&state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    if let Ok(val) = HeaderValue::from_str(&clear_refresh_cookie_value(&state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    set_request_id_header(&mut response_headers, &request_id);
    Ok((
        StatusCode::OK,
        response_headers,
        Json(json!({ "status": "ok" })),
    ))
}

pub(crate) async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    let token = cookie_value(&headers, &state.config.auth_cookie_name)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "NO_AUTH_COOKIE"))?;

    let mut validation = jsonwebtoken::Validation::new(Algorithm::HS256);
    validation.validate_exp = false;
    validation.validate_aud = false;
    let claims = jsonwebtoken::decode::<crate::gateway::AuthClaims>(
        &token,
        &jsonwebtoken::DecodingKey::from_secret(state.config.auth_jwt_secret.as_bytes()),
        &validation,
    )
    .map_err(|_| api_error(StatusCode::UNAUTHORIZED, "INVALID_TOKEN"))?
    .claims;

    let user = state
        .user_store
        .find_by_id(&claims.sub)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "USER_NOT_FOUND"))?;

    let mut response_headers = HeaderMap::new();
    set_request_id_header(&mut response_headers, &request_id);
    Ok((StatusCode::OK, response_headers, Json(user_response(&user))))
}

pub(crate) async fn oidc_start(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if !state.config.oidc_enabled {
        return Err(api_error(StatusCode::NOT_FOUND, "OIDC_DISABLED"));
    }

    let client = build_oidc_client(&state)?;
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut auth_request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(pkce_challenge);
    for scope in state.config.oidc_scopes.split_whitespace() {
        auth_request = auth_request.add_scope(Scope::new(scope.to_string()));
    }
    auth_request = auth_request.add_extra_param("nonce", Uuid::new_v4().to_string());
    let (auth_url, csrf_token) = auth_request.url();

    let expires_at = crate::runtime::now_secs() + state.config.oidc_state_ttl_secs;
    state.oidc_states.lock().expect("oidc state lock").insert(
        csrf_token.secret().to_string(),
        crate::gateway::OidcStateRecord {
            verifier: pkce_verifier.secret().to_string(),
            expires_at_secs: expires_at,
        },
    );

    let mut response_headers = HeaderMap::new();
    set_request_id_header(&mut response_headers, &request_id);
    Ok((
        StatusCode::TEMPORARY_REDIRECT,
        response_headers,
        axum::response::Redirect::temporary(auth_url.as_ref()),
    ))
}

pub(crate) async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<OidcCallbackQuery>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if !state.config.oidc_enabled {
        return Err(api_error(StatusCode::NOT_FOUND, "OIDC_DISABLED"));
    }
    let client = build_oidc_client(&state)?;
    let oidc_state = state
        .oidc_states
        .lock()
        .expect("oidc state lock")
        .remove(&query.state)
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "OIDC_STATE_INVALID"))?;
    if oidc_state.expires_at_secs <= crate::runtime::now_secs() {
        return Err(api_error(StatusCode::BAD_REQUEST, "OIDC_STATE_EXPIRED"));
    }

    let token_response = client
        .exchange_code(AuthorizationCode::new(query.code))
        .set_pkce_verifier(PkceCodeVerifier::new(oidc_state.verifier))
        .request_async(async_http_client)
        .await
        .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "OIDC_TOKEN_EXCHANGE_FAILED"))?;

    let access_token = token_response.access_token().secret().to_string();
    let profile = if !state.config.oidc_userinfo_url.is_empty() {
        state
            .log_client
            .get(&state.config.oidc_userinfo_url)
            .bearer_auth(&access_token)
            .send()
            .await
            .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "OIDC_USERINFO_FAILED"))?
            .json::<Value>()
            .await
            .map_err(|_| api_error(StatusCode::BAD_GATEWAY, "OIDC_USERINFO_FAILED"))?
    } else {
        json!({})
    };

    let sub = profile
        .get("sub")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let email = profile
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_lowercase)
        .unwrap_or_else(|| format!("{}@oidc.local", sub));
    let name = profile
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| email.clone());

    let user = if let Some(existing) = state.user_store.find_by_email(&email) {
        existing
    } else {
        let now = crate::runtime::now_millis();
        let created = UserRecord {
            id: format!("oidc:{}", sub),
            email,
            password_hash: hash_password(&Uuid::new_v4().to_string())
                .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "HASH_FAILED"))?,
            name,
            role: "user".to_string(),
            status: "active".to_string(),
            email_verified: true,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            refresh_token_hash: None,
            refresh_token_expires_at: None,
            verification_token_hash: None,
            verification_token_expires_at: None,
            reset_token_hash: None,
            reset_token_expires_at: None,
        };
        state
            .user_store
            .create(created)
            .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_USER_CREATE_FAILED"))?
    };

    let _ = state.user_store.update_login(&user.id);
    let user = state.user_store.find_by_id(&user.id).unwrap_or(user);
    let jwt = issue_jwt(&user, &state)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "JWT_FAILED"))?;

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&auth_cookie_value(&jwt, &state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    let _ = issue_refresh_cookie_for_user(&state, &user, &mut response_headers);
    set_request_id_header(&mut response_headers, &request_id);
    Ok((StatusCode::OK, response_headers, Json(user_response(&user))))
}

pub(crate) async fn refresh(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if !state.config.auth_refresh_enabled {
        return Err(api_error(StatusCode::NOT_FOUND, "REFRESH_DISABLED"));
    }
    let token = cookie_value(&headers, &state.config.refresh_cookie_name)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "NO_REFRESH_COOKIE"))?;
    let token_hash = hash_token(&state.config.auth_jwt_secret, &token);
    let user = state
        .user_store
        .find_by_refresh_token_hash(&token_hash)
        .ok_or_else(|| api_error(StatusCode::UNAUTHORIZED, "INVALID_REFRESH_TOKEN"))?;

    let jwt = issue_jwt(&user, &state)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "JWT_FAILED"))?;
    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&auth_cookie_value(&jwt, &state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    issue_refresh_cookie_for_user(&state, &user, &mut response_headers)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "REFRESH_ROTATE_FAILED"))?;
    set_request_id_header(&mut response_headers, &request_id);
    Ok((StatusCode::OK, response_headers, Json(user_response(&user))))
}

pub(crate) async fn forgot_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ForgotPasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    let email = body.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "EMAIL_REQUIRED"));
    }
    let mut response_headers = HeaderMap::new();
    set_request_id_header(&mut response_headers, &request_id);

    let token = random_token();
    let token_hash = hash_token(&state.config.auth_jwt_secret, &token);
    let expires_at = crate::runtime::now_millis() + state.config.password_reset_ttl_ms as i64;
    let user_opt = state
        .user_store
        .issue_reset_token_by_email(&email, token_hash, expires_at)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "RESET_TOKEN_FAILED"))?;
    if let Some(user) = user_opt {
        let _ = state
            .backend
            .auth_outbox_event(
                "password_reset",
                &user.email,
                json!({
                    "token": token,
                    "expiresAt": expires_at,
                    "userId": user.id,
                    "name": user.name
                }),
                &crate::adapters::backend::BackendContext {
                    site: "default".to_string(),
                    actor_key: Some(format!("user:{}", user.id)),
                    connection_id: None,
                    ip: Some(crate::gateway::connection_ip_from_headers(&headers)),
                    user_agent: headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    user: Some(crate::adapters::backend::BackendUser {
                        id: Some(user.id.clone()),
                        role: user.role.clone(),
                    }),
                },
                Some(&request_id),
            )
            .await;
    }

    Ok((
        StatusCode::OK,
        response_headers,
        Json(json!({ "status": "ok" })),
    ))
}

pub(crate) async fn reset_password(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ResetPasswordRequest>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if body.new_password.len() < 8 {
        return Err(api_error(StatusCode::BAD_REQUEST, "PASSWORD_TOO_SHORT"));
    }
    if body.token.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "TOKEN_REQUIRED"));
    }
    let token_hash = hash_token(&state.config.auth_jwt_secret, body.token.trim());
    let next_hash = hash_password(&body.new_password)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "HASH_FAILED"))?;
    let updated = state
        .user_store
        .reset_password_by_token_hash(&token_hash, next_hash)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "RESET_FAILED"))?;
    if updated.is_none() {
        return Err(api_error(StatusCode::BAD_REQUEST, "RESET_TOKEN_INVALID"));
    }

    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&clear_refresh_cookie_value(&state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    set_request_id_header(&mut response_headers, &request_id);
    Ok((
        StatusCode::OK,
        response_headers,
        Json(json!({ "status": "ok" })),
    ))
}

pub(crate) async fn verify_email(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<VerifyEmailRequest>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    if body.token.trim().is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "TOKEN_REQUIRED"));
    }
    let token_hash = hash_token(&state.config.auth_jwt_secret, body.token.trim());
    let user = state
        .user_store
        .mark_email_verified_by_token_hash(&token_hash)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "VERIFY_FAILED"))?
        .ok_or_else(|| api_error(StatusCode::BAD_REQUEST, "VERIFY_TOKEN_INVALID"))?;

    let jwt = issue_jwt(&user, &state)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "JWT_FAILED"))?;
    let mut response_headers = HeaderMap::new();
    if let Ok(val) = HeaderValue::from_str(&auth_cookie_value(&jwt, &state.config)) {
        response_headers.append(SET_COOKIE, val);
    }
    let _ = issue_refresh_cookie_for_user(&state, &user, &mut response_headers);
    set_request_id_header(&mut response_headers, &request_id);
    Ok((StatusCode::OK, response_headers, Json(user_response(&user))))
}

pub(crate) async fn resend_verification(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<ResendVerificationRequest>,
) -> ApiResult<impl IntoResponse> {
    let request_id = crate::gateway::request_id_from_headers(&headers);
    let email = body.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(api_error(StatusCode::BAD_REQUEST, "EMAIL_REQUIRED"));
    }
    let mut response_headers = HeaderMap::new();
    set_request_id_header(&mut response_headers, &request_id);

    let token = random_token();
    let token_hash = hash_token(&state.config.auth_jwt_secret, &token);
    let expires_at = crate::runtime::now_millis() + state.config.email_verify_ttl_ms as i64;
    let user_opt = state
        .user_store
        .issue_verification_token_by_email(&email, token_hash, expires_at)
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "VERIFY_RESEND_FAILED"))?;
    if let Some(user) = user_opt {
        let _ = state
            .backend
            .auth_outbox_event(
                "verify_email",
                &user.email,
                json!({
                    "token": token,
                    "expiresAt": expires_at,
                    "userId": user.id,
                    "name": user.name
                }),
                &crate::adapters::backend::BackendContext {
                    site: "default".to_string(),
                    actor_key: Some(format!("user:{}", user.id)),
                    connection_id: None,
                    ip: Some(crate::gateway::connection_ip_from_headers(&headers)),
                    user_agent: headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string),
                    user: Some(crate::adapters::backend::BackendUser {
                        id: Some(user.id.clone()),
                        role: user.role.clone(),
                    }),
                },
                Some(&request_id),
            )
            .await;
    }

    Ok((
        StatusCode::OK,
        response_headers,
        Json(json!({ "status": "ok" })),
    ))
}

fn build_oidc_client(state: &AppState) -> ApiResult<BasicClient> {
    let auth_url = AuthUrl::new(state.config.oidc_auth_url.clone())
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_CONFIG_INVALID"))?;
    let token_url = TokenUrl::new(state.config.oidc_token_url.clone())
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_CONFIG_INVALID"))?;
    let redirect_url = RedirectUrl::new(state.config.oidc_redirect_url.clone())
        .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_CONFIG_INVALID"))?;
    let mut client = BasicClient::new(
        ClientId::new(state.config.oidc_client_id.clone()),
        Some(ClientSecret::new(state.config.oidc_client_secret.clone())),
        auth_url,
        Some(token_url),
    )
    .set_redirect_uri(redirect_url);
    if state.config.oidc_client_secret.is_empty() {
        client = BasicClient::new(
            ClientId::new(state.config.oidc_client_id.clone()),
            None,
            AuthUrl::new(state.config.oidc_auth_url.clone())
                .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_CONFIG_INVALID"))?,
            Some(
                TokenUrl::new(state.config.oidc_token_url.clone()).map_err(|_| {
                    api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_CONFIG_INVALID")
                })?,
            ),
        )
        .set_redirect_uri(
            RedirectUrl::new(state.config.oidc_redirect_url.clone())
                .map_err(|_| api_error(StatusCode::INTERNAL_SERVER_ERROR, "OIDC_CONFIG_INVALID"))?,
        );
    }
    Ok(client)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_password() {
        let password = "secure-p@ssw0rd!";
        let hash = hash_password(password).expect("hashing should succeed");
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn hash_generates_unique_salts() {
        let password = "same-password";
        let hash1 = hash_password(password).unwrap();
        let hash2 = hash_password(password).unwrap();
        // Different salts → different hashes
        assert_ne!(hash1, hash2);
        // But both verify correctly
        assert!(verify_password(password, &hash1));
        assert!(verify_password(password, &hash2));
    }

    #[test]
    fn verify_rejects_invalid_hash_format() {
        assert!(!verify_password("password", "not-a-valid-hash"));
    }

    #[test]
    fn user_to_json_excludes_password_hash() {
        let user = UserRecord {
            id: "u1".into(),
            email: "a@b.com".into(),
            password_hash: "secret".into(),
            name: "Test".into(),
            role: "user".into(),
            status: "active".into(),
            email_verified: true,
            created_at: 0,
            updated_at: 0,
            last_login_at: None,
            refresh_token_hash: None,
            refresh_token_expires_at: None,
            verification_token_hash: None,
            verification_token_expires_at: None,
            reset_token_hash: None,
            reset_token_expires_at: None,
        };
        let json = user_to_json(&user);
        assert!(json.get("passwordHash").is_none());
        assert!(json.get("password_hash").is_none());
        assert_eq!(json["email"], "a@b.com");
    }
}
