pub mod models;

use chrono::{DateTime, Local, NaiveDate};

use crate::config::OrgHeader;
use models::{Issue, User, WorklogPost, WorklogRecord};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("API {status}: {msg}")]
    Status { status: u16, msg: String },
    #[error("{0}")]
    Other(String),
}

impl ApiError {
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, ApiError::Status { status: 401, .. })
    }

    pub async fn from_response(status: reqwest::StatusCode, body: String) -> Self {
        let msg = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
                    .or_else(|| {
                        // `errors` arrives either as an array of strings or
                        // as an object of field → message
                        let e = v.get("errors")?;
                        let parts: Vec<String> = if let Some(arr) = e.as_array() {
                            arr.iter()
                                .filter_map(|x| x.as_str())
                                .map(str::to_string)
                                .collect()
                        } else if let Some(obj) = e.as_object() {
                            obj.values()
                                .filter_map(|x| x.as_str())
                                .map(str::to_string)
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let joined = parts.join("; ");
                        (!joined.is_empty()).then_some(joined)
                    })
            })
            .unwrap_or(if body.is_empty() {
                status.canonical_reason().unwrap_or("error").to_string()
            } else {
                body
            });
        ApiError::Status {
            status: status.as_u16(),
            msg,
        }
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Thin client for the Yandex Tracker API.
#[derive(Clone)]
pub struct TrackerClient {
    http: reqwest::Client,
    base: String,
    token: String,
    org_id: String,
    header: OrgHeader,
}

impl TrackerClient {
    pub fn new(token: String, org_id: String, header: OrgHeader, base: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .user_agent(concat!("ytime/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
            base,
            token,
            org_id,
            header,
        }
    }

    fn auth_value(&self, token: &str) -> String {
        format!("OAuth {token}")
    }

    async fn send(&self, req: reqwest::RequestBuilder) -> ApiResult<reqwest::Response> {
        let resp = req.send().await?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(ApiError::from_response(status, body).await)
        }
    }

    pub async fn myself(&self) -> ApiResult<User> {
        let token = self.token.clone();
        let req = self
            .http
            .get(format!("{}/v2/myself", self.base))
            .header("Authorization", self.auth_value(&token))
            .header(self.header.header_name(), &self.org_id);
        Ok(self.send(req).await?.json().await?)
    }

    /// `GET /v2/issues?query=...` — used both for `assignee:me()` and free-text
    /// search. A query that is exactly an issue key (`ADS-2657`) is also
    /// fetched directly and pinned to the top: the text search tokenizes keys
    /// on the hyphen and returns lookalikes instead of the issue itself.
    pub async fn issues(&self, query: &str, max_pages: u32) -> ApiResult<Vec<Issue>> {
        let token = self.token.clone();
        let mut exact: Vec<Issue> = Vec::new();
        if let Some(key) = issue_key_form(query) {
            let req = self
                .http
                .get(format!("{}/v2/issues/{key}", self.base))
                .header("Authorization", self.auth_value(&token))
                .header(self.header.header_name(), &self.org_id);
            match self.send(req).await {
                Ok(resp) => match resp.json::<Issue>().await {
                    Ok(issue) => exact.push(issue),
                    Err(e) => return Err(e.into()),
                },
                // 404 or similar: fall through to the text search below
                Err(e) if !e.is_unauthorized() => {}
                Err(e) => return Err(e),
            }
        }
        let mut all = Vec::new();
        for page in 1..=max_pages {
            let req = self
                .http
                .get(format!("{}/v2/issues", self.base))
                .header("Authorization", self.auth_value(&token))
                .header(self.header.header_name(), &self.org_id)
                .query(&[
                    ("query", query),
                    ("perPage", "100"),
                    ("page", &page.to_string()),
                ]);
            let issues: Vec<Issue> = match self.send(req).await {
                Ok(resp) => resp.json().await?,
                // with the exact issue in hand the search served its purpose
                Err(e) if !exact.is_empty() && !e.is_unauthorized() => break,
                Err(e) => return Err(e),
            };
            let got = issues.len();
            all.extend(issues);
            if got < 100 {
                break;
            }
        }
        // exact key first, then text matches, deduplicated
        for issue in all {
            if !exact.iter().any(|i| i.key == issue.key) {
                exact.push(issue);
            }
        }
        Ok(exact)
    }

    pub async fn add_worklog(
        &self,
        issue_key: &str,
        start: DateTime<Local>,
        duration: std::time::Duration,
        comment: &str,
    ) -> ApiResult<()> {
        let token = self.token.clone();
        let body = WorklogPost {
            // Tracker's own wire format, e.g. `2026-10-20T11:45:00.123+0300`.
            start: start.format("%Y-%m-%dT%H:%M:%S%.3f%z").to_string(),
            duration: crate::timer::format_iso_duration(duration),
            comment: if comment.trim().is_empty() {
                None
            } else {
                Some(comment.trim().to_string())
            },
        };
        let req = self
            .http
            .post(format!("{}/v2/issues/{issue_key}/worklog", self.base))
            .header("Authorization", self.auth_value(&token))
            .header(self.header.header_name(), &self.org_id)
            .json(&body);
        self.send(req).await?;
        Ok(())
    }

    /// `POST /v3/worklog/_search` — records authored by `login`, created in
    /// `[from, to]`. Paginates via query params (the server's default page
    /// size is only 50 records).
    pub async fn worklog_search(
        &self,
        login: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> ApiResult<Vec<WorklogRecord>> {
        let token = self.token.clone();
        // Docs' example format: `2018-06-06T08:42:15.258`, local time, no offset.
        let fmt = |d: NaiveDate| format!("{}T00:00:00.000", d.format("%Y-%m-%d"));
        let body = serde_json::json!({
            "createdBy": login,
            "createdAt": { "from": fmt(from), "to": fmt(to) },
        });
        let mut all = Vec::new();
        for page in 1..=10u32 {
            let req = self
                .http
                .post(format!("{}/v3/worklog/_search", self.base))
                .header("Authorization", self.auth_value(&token))
                .header(self.header.header_name(), &self.org_id)
                .query(&[("perPage", "100"), ("page", &page.to_string())])
                .json(&body);
            let records: Vec<WorklogRecord> = self.send(req).await?.json().await?;
            let got = records.len();
            all.extend(records);
            if got < 100 {
                break;
            }
        }
        Ok(all)
    }
}

/// Probes both org headers and returns the one the API accepts, plus the
/// authenticated user. Used on first connect and after token changes.
/// `Some(normalized_key)` when `q` is exactly an issue key (`ADS-2657`),
/// case-insensitively.
fn issue_key_form(q: &str) -> Option<String> {
    let q = q.trim();
    let (prefix, number) = q.split_once('-')?;
    if prefix.is_empty() || prefix.len() > 16 || number.is_empty() || number.len() > 8 {
        return None;
    }
    if !prefix.chars().all(|c| c.is_ascii_alphabetic())
        || !number.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{}-{number}", prefix.to_uppercase()))
}

pub async fn detect_org_header(
    cfg: &crate::config::Config,
    token: String,
) -> ApiResult<(OrgHeader, User)> {
    let candidates = match cfg.org_header {
        Some(h) => vec![h],
        None => vec![OrgHeader::XOrgId, OrgHeader::XCloudOrgId],
    };
    let org_id = cfg
        .org_id
        .clone()
        .ok_or_else(|| ApiError::Other("org id is not configured".into()))?;
    let mut last_err = ApiError::Other("no candidates".into());
    for header in candidates {
        let probe = TrackerClient::new(token.clone(), org_id.clone(), header, cfg.api_base.clone());
        match probe.issues("assignee:me()", 1).await {
            Ok(_) => {
                let user = probe.myself().await?;
                return Ok((header, user));
            }
            Err(e) if e.is_unauthorized() => return Err(e),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn error_message_from_tracker_body() {
        let e = ApiError::from_response(
            reqwest::StatusCode::NOT_FOUND,
            r#"{"errors":["Issue not found"]}"#.into(),
        )
        .await;
        assert!(matches!(e, ApiError::Status { status: 404, ref msg } if msg == "Issue not found"));
        assert!(!e.is_unauthorized());
    }

    #[test]
    fn issue_key_detection() {
        assert_eq!(issue_key_form("ADS-2657").as_deref(), Some("ADS-2657"));
        assert_eq!(issue_key_form(" ads-2657 ").as_deref(), Some("ADS-2657"));
        assert_eq!(issue_key_form("CI-2").as_deref(), Some("CI-2"));
        // not keys: text queries, multiple segments, empty parts
        assert_eq!(issue_key_form("kafka lag"), None);
        assert_eq!(issue_key_form("ADS-2657-extra"), None);
        assert_eq!(issue_key_form("-2657"), None);
        assert_eq!(issue_key_form("ADS-"), None);
        assert_eq!(issue_key_form("assignee:me()"), None);
    }
}
