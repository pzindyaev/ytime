use serde::Deserialize;

use crate::store::Tokens;

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("{0}")]
    Oauth(String),
}

impl AuthError {
    fn from_body(body: &str) -> Self {
        let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
        let code = parsed
            .as_ref()
            .and_then(|v| v.get("error").and_then(|e| e.as_str()))
            .map(str::to_string);
        match code {
            Some(code) => {
                let desc = parsed
                    .as_ref()
                    .and_then(|v| v.get("error_description").and_then(|d| d.as_str()))
                    .filter(|d| !d.is_empty());
                AuthError::Oauth(match desc {
                    Some(d) => format!("{code} ({d})"),
                    None => code,
                })
            }
            None => AuthError::Oauth(if body.is_empty() {
                "unknown error".to_string()
            } else {
                body.to_string()
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    #[allow(dead_code)]
    pub expires_in: Option<u64>,
}

fn default_interval() -> u64 {
    5
}

#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

impl From<TokenResponse> for Tokens {
    fn from(t: TokenResponse) -> Self {
        Tokens {
            access_token: t.access_token,
            refresh_token: t.refresh_token,
        }
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("ytime/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .expect("reqwest client")
}

/// Step 1 of the device flow: ask Yandex for a user code to confirm in the browser.
pub async fn request_device_code(
    oauth_base: &str,
    client_id: &str,
) -> Result<DeviceCode, AuthError> {
    let resp = http()
        .post(format!("{oauth_base}/device/code"))
        .form(&[("client_id", client_id)])
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if status.is_success() {
        serde_json::from_str(&body).map_err(|e| AuthError::Oauth(format!("bad response: {e}")))
    } else {
        Err(AuthError::from_body(&body))
    }
}

/// Outcome of a single token poll in the device flow.
#[derive(Debug)]
pub enum PollOutcome {
    Token(TokenResponse),
    Pending,
    /// The code expired — the flow must restart.
    Expired,
    /// The user denied access.
    Denied,
}

/// One polling attempt of step 2 of the device flow.
pub async fn poll_token(
    oauth_base: &str,
    client_id: &str,
    client_secret: Option<&str>,
    device_code: &str,
) -> Result<PollOutcome, AuthError> {
    let mut form = vec![
        ("grant_type", "device_code"),
        ("code", device_code),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let resp = http()
        .post(format!("{oauth_base}/token"))
        .form(&form)
        .send()
        .await?;
    let body = resp.text().await?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AuthError::Oauth(format!("bad response: {e}")))?;
    if let Some(err) = parsed.get("error").and_then(|e| e.as_str()) {
        if err != "authorization_pending" && err != "invalid_grant" && err != "access_denied" {
            return Err(AuthError::from_body(&body));
        }
        return Ok(match err {
            "authorization_pending" => PollOutcome::Pending,
            "invalid_grant" => PollOutcome::Expired,
            "access_denied" => PollOutcome::Denied,
            _ => unreachable!(),
        });
    }
    let access_token = parsed
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| AuthError::Oauth("no access_token in response".into()))?
        .to_string();
    let refresh_token = parsed
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    Ok(PollOutcome::Token(TokenResponse {
        access_token,
        refresh_token,
    }))
}

/// Refresh an access token with the stored refresh token.
pub async fn refresh(
    oauth_base: &str,
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<TokenResponse, AuthError> {
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let resp = http()
        .post(format!("{oauth_base}/token"))
        .form(&form)
        .send()
        .await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        return Err(AuthError::from_body(&body));
    }
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| AuthError::Oauth(format!("bad response: {e}")))?;
    if let Some(err) = parsed.get("error").and_then(|e| e.as_str()) {
        return Err(AuthError::Oauth(err.to_string()));
    }
    let access_token = parsed
        .get("access_token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| AuthError::Oauth("no access_token in response".into()))?
        .to_string();
    let new_refresh = parsed
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(str::to_string);
    Ok(TokenResponse {
        access_token,
        refresh_token: new_refresh.or(Some(refresh_token.to_string())),
    })
}

/// Console device flow for the `ytime auth` subcommand: prints the code,
/// polls until the user confirms, returns the tokens.
pub async fn run_console_flow(
    oauth_base: &str,
    client_id: &str,
    client_secret: Option<&str>,
) -> Result<TokenResponse, AuthError> {
    let code = request_device_code(oauth_base, client_id).await?;
    println!("Open {} and enter the code:", code.verification_url);
    println!();
    println!("    {}", code.user_code);
    println!();
    println!("Waiting for authorization…");
    let deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(code.expires_in.unwrap_or(600));
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(code.interval.max(1))).await;
        match poll_token(oauth_base, client_id, client_secret, &code.device_code).await? {
            PollOutcome::Token(t) => return Ok(t),
            PollOutcome::Pending => {}
            PollOutcome::Expired => return Err(AuthError::Oauth("code expired, restart".into())),
            PollOutcome::Denied => return Err(AuthError::Oauth("access denied".into())),
        }
        if std::time::Instant::now() > deadline {
            return Err(AuthError::Oauth("code expired, restart".into()));
        }
    }
}
