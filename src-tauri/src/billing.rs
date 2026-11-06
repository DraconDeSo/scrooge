//! Opt-in billing connections. Configuration stores environment variable names only.
use crate::settings::Settings;
use chrono::{Datelike, Days, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

const DAY_MS: u64 = 86_400_000;
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_PAGES: usize = 50;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ChargeKind {
    Api,
    PlanExtra,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Charge {
    pub currency: String,
    pub kind: ChargeKind,
    pub amount_micros: i64,
}

#[derive(Clone)]
pub struct BilledRow {
    pub project: Option<String>,
    pub model: Option<String>,
    pub charge: Charge,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Period {
    start: u64,
    end: u64,
}

impl Period {
    pub fn new(start: u64, end: u64) -> Option<Self> {
        let tomorrow = (Utc::now().timestamp_millis().max(0) as u64 / DAY_MS + 1) * DAY_MS;
        let end = end.min(tomorrow);
        (start < end
            && start.is_multiple_of(DAY_MS)
            && end.is_multiple_of(DAY_MS)
            && end - start <= 366 * DAY_MS)
            .then_some(Self { start, end })
    }

    fn date(ms: u64) -> NaiveDate {
        chrono::DateTime::from_timestamp_millis(ms as i64)
            .unwrap()
            .date_naive()
    }

    fn dates(self) -> (NaiveDate, NaiveDate) {
        (Self::date(self.start), Self::date(self.end))
    }
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
struct Account {
    account: String,
    key_env: Option<String>,
    #[serde(default)]
    project_paths: BTreeMap<String, String>,
    #[serde(default)]
    model_names: BTreeMap<String, String>,
    #[serde(flatten)]
    source: Provider,
}

#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
enum Provider {
    Openai,
    Anthropic,
    Github {
        owner: String,
        scope: GithubScope,
        #[serde(default)]
        legacy_premium_requests: bool,
    },
    Xai {
        team_id: String,
    },
    Google {
        query_project: String,
        table: String,
        location: String,
        service_ids: Vec<String>,
        #[serde(default)]
        sku_ids: Vec<String>,
        maximum_bytes_billed: u64,
    },
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GithubScope {
    User,
    Organization,
    Enterprise,
}

impl Provider {
    fn id(&self) -> &'static str {
        match self {
            Self::Openai => "codex",
            Self::Anthropic => "claude",
            Self::Github { .. } => "copilot",
            Self::Xai { .. } => "grok",
            Self::Google { .. } => "gemini",
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Openai => "OpenAI API",
            Self::Anthropic => "Anthropic Console",
            Self::Github { .. } => "GitHub Copilot",
            Self::Xai { .. } => "xAI",
            Self::Google { .. } => "Google Cloud",
        }
    }

    fn kind(&self) -> ChargeKind {
        if matches!(self, Self::Github { .. }) {
            ChargeKind::PlanExtra
        } else {
            ChargeKind::Api
        }
    }
}

#[derive(Clone)]
pub struct ConnectionReport {
    pub account: String,
    pub provider: &'static str,
    pub label: &'static str,
    pub kind: ChargeKind,
    pub result: Result<Vec<BilledRow>, String>,
}

#[derive(Default)]
pub struct SyncReport {
    pub connections: Vec<ConnectionReport>,
    pub status: Option<&'static str>,
}

impl SyncReport {
    pub fn replaces_import(&self, provider: &str, account: Option<&str>, kind: ChargeKind) -> bool {
        self.connections.iter().any(|entry| {
            entry.provider == provider
                && entry.kind == kind
                && entry.result.is_ok()
                && account.is_some_and(|name| entry.account.eq_ignore_ascii_case(name))
        })
    }
}

struct Cached {
    account: Account,
    period: Period,
    at: Instant,
    report: ConnectionReport,
}

#[derive(Default)]
pub struct Sync {
    entries: Vec<Cached>,
}

impl Sync {
    pub fn report(
        &mut self,
        file: &Path,
        settings: &Settings,
        period: Option<Period>,
    ) -> SyncReport {
        self.entries.retain(|entry| entry.at.elapsed() < CACHE_TTL);
        let mut report = SyncReport::default();
        let configs = match load_accounts(file) {
            Ok(configs) if !configs.is_empty() => configs,
            Ok(_) => {
                report.status = Some("Configure billing-sources.json in the usage imports folder");
                return report;
            }
            Err(error) => {
                report.status = Some(error);
                return report;
            }
        };
        let Some(period) = period else {
            report.status = Some("Select up to 366 calendar days for provider billing");
            return report;
        };
        let mut seen = HashSet::new();
        for account in configs
            .into_iter()
            .filter(|a| settings.provider_enabled(a.source.id()))
        {
            if !seen.insert((account.source.id(), account.account.to_ascii_lowercase())) {
                report.status = Some("Billing account names must be unique per provider");
                continue;
            }
            if let Some(cached) = self
                .entries
                .iter()
                .find(|e| e.account == account && e.period == period)
            {
                report.connections.push(cached.report.clone());
                continue;
            }
            let connection = ConnectionReport {
                provider: account.source.id(),
                label: account.source.label(),
                account: account.account.clone(),
                kind: account.source.kind(),
                result: account.fetch(period),
            };
            if self.entries.len() >= 32 {
                self.entries.remove(0);
            }
            self.entries.push(Cached {
                account,
                period,
                at: Instant::now(),
                report: connection.clone(),
            });
            report.connections.push(connection);
        }
        report
    }
}

fn load_accounts(path: &Path) -> Result<Vec<Account>, &'static str> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("Cannot read billing-sources.json"),
    };
    let mut data = Vec::new();
    file.take(65_537)
        .read_to_end(&mut data)
        .map_err(|_| "Cannot read billing-sources.json")?;
    if data.len() > 65_536 {
        return Err("billing-sources.json exceeds 64 KB");
    }
    let accounts: Vec<Account> =
        serde_json::from_slice(data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data))
            .map_err(|_| "Invalid billing-sources.json")?;
    if accounts.len() > 16
        || accounts
            .iter()
            .any(|a| a.account.trim().is_empty() || a.account.len() > 128)
    {
        return Err("Configure at most 16 billing accounts with valid labels");
    }
    Ok(accounts)
}

impl Account {
    fn credential(&self) -> Result<String, String> {
        let credential = match self.key_env.as_deref() {
            Some(name)
                if name.starts_with("SCROOGE_BILLING_")
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_') =>
            {
                std::env::var(name).ok()
            }
            None if matches!(self.source, Provider::Google { .. }) => {
                let gcloud = if cfg!(windows) {
                    "gcloud.cmd"
                } else {
                    "gcloud"
                };
                let output = crate::quota::hidden_command(gcloud)
                    .args([
                        "auth",
                        "application-default",
                        "print-access-token",
                        "--quiet",
                    ])
                    .output()
                    .map_err(|_| {
                        "Install the Google Cloud CLI for Application Default Credentials"
                    })?;
                if !output.status.success() {
                    return Err("Configure Google Application Default Credentials".into());
                }
                String::from_utf8(output.stdout)
                    .ok()
                    .map(|s| s.trim().to_owned())
            }
            _ => {
                return Err("Use a SCROOGE_BILLING_ environment variable for the credential".into())
            }
        };
        credential
            .filter(|s| !s.is_empty() && s.len() <= 8192 && s.bytes().all(|b| b.is_ascii_graphic()))
            .ok_or_else(|| "Billing credential is missing or invalid".into())
    }

    fn fetch(&self, period: Period) -> Result<Vec<BilledRow>, String> {
        let credential = self.credential()?;
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(18)))
            .build()
            .new_agent();
        match &self.source {
            Provider::Openai => self.openai(&agent, &credential, period),
            Provider::Anthropic => self.anthropic(&agent, &credential, period),
            Provider::Github {
                owner,
                scope,
                legacy_premium_requests,
            } => self.github(
                &agent,
                &credential,
                period,
                owner,
                *scope,
                *legacy_premium_requests,
            ),
            Provider::Xai { team_id } => self.xai(&agent, &credential, period, team_id),
            Provider::Google {
                query_project,
                table,
                location,
                service_ids,
                sku_ids,
                maximum_bytes_billed,
            } => {
                let query = google_query(
                    period,
                    table,
                    location,
                    service_ids,
                    sku_ids,
                    *maximum_bytes_billed,
                )?;
                if !identifier(query_project) {
                    return Err("Invalid Google billing query project".into());
                }
                self.google(&agent, &credential, query_project, location, &query)
            }
        }
    }

    fn row(
        &self,
        project: &Value,
        model: Option<&str>,
        currency: &str,
        amount_micros: i64,
    ) -> Result<BilledRow, String> {
        let currency = currency.to_ascii_uppercase();
        if currency.len() != 3 || !currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err("Invalid currency in provider response".into());
        }
        Ok(BilledRow {
            project: project
                .as_str()
                .and_then(|id| self.project_paths.get(id))
                .cloned(),
            model: model.filter(|s| !s.is_empty()).map(str::to_owned),
            charge: Charge {
                currency,
                amount_micros,
                kind: self.source.kind(),
            },
        })
    }

    fn openai(
        &self,
        agent: &ureq::Agent,
        key: &str,
        period: Period,
    ) -> Result<Vec<BilledRow>, String> {
        let mut query = vec![
            ("start_time", (period.start / 1000).to_string()),
            (
                "end_time",
                (period.end / 1000)
                    .min(Utc::now().timestamp().max(0) as u64)
                    .to_string(),
            ),
            ("bucket_width", "1d".into()),
            ("limit", "180".into()),
            ("group_by", "project_id".into()),
            ("group_by", "line_item".into()),
        ];
        let mut rows = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = get(
                agent,
                "https://api.openai.com/v1/organization/costs",
                &query,
                &[("Authorization", &format!("Bearer {key}"))],
            )?;
            for bucket in array(&page["data"])? {
                for item in array(&bucket["results"])? {
                    let amount = &item["amount"];
                    let model = item["line_item"]
                        .as_str()
                        .and_then(|line| self.openai_model(line));
                    rows.push(
                        self.row(
                            &item["project_id"],
                            model,
                            amount["currency"]
                                .as_str()
                                .ok_or("Missing OpenAI currency")?,
                            money(&amount["value"], 1_000_000.0)?,
                        )?,
                    );
                }
            }
            let Some(next) = next_page(&page)? else {
                return Ok(rows);
            };
            replace_page(&mut query, "page", next);
        }
        Err("OpenAI billing pagination limit reached".into())
    }

    fn openai_model<'a>(&'a self, line: &'a str) -> Option<&'a str> {
        self.model_names.get(line).map(String::as_str).or_else(|| {
            let (model, unit) = line.split_once(", ")?;
            matches!(
                unit,
                "input_tokens" | "output_tokens" | "cached_input_tokens" | "input_cached_tokens"
            )
            .then_some(model)
        })
    }

    fn anthropic(
        &self,
        agent: &ureq::Agent,
        key: &str,
        period: Period,
    ) -> Result<Vec<BilledRow>, String> {
        let (start, end) = period.dates();
        let mut query = vec![
            ("starting_at", format!("{start}T00:00:00Z")),
            ("ending_at", format!("{end}T00:00:00Z")),
            ("bucket_width", "1d".into()),
            ("limit", "31".into()),
            ("group_by[]", "workspace_id".into()),
            ("group_by[]", "description".into()),
        ];
        let mut rows = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = get(
                agent,
                "https://api.anthropic.com/v1/organizations/cost_report",
                &query,
                &[("x-api-key", key), ("anthropic-version", "2026-10-21")],
            )?;
            for bucket in array(&page["data"])? {
                for item in array(&bucket["results"])? {
                    // The Console cost report expresses decimal strings in cents.
                    rows.push(
                        self.row(
                            &item["workspace_id"],
                            item["model"].as_str(),
                            item["currency"]
                                .as_str()
                                .ok_or("Missing Anthropic currency")?,
                            money(&item["amount"], 10_000.0)?,
                        )?,
                    );
                }
            }
            let Some(next) = next_page(&page)? else {
                return Ok(rows);
            };
            replace_page(&mut query, "page", next);
        }
        Err("Anthropic billing pagination limit reached".into())
    }

    fn github(
        &self,
        agent: &ureq::Agent,
        key: &str,
        period: Period,
        owner: &str,
        scope: GithubScope,
        legacy_premium_requests: bool,
    ) -> Result<Vec<BilledRow>, String> {
        if !identifier(owner) {
            return Err("Invalid GitHub billing owner".into());
        }
        let path = match scope {
            GithubScope::User => "users",
            GithubScope::Organization => "organizations",
            GithubScope::Enterprise => "enterprises",
        };
        let mut rows = Vec::new();
        for (day, full_month) in github_periods(period) {
            let report = github_report(day, legacy_premium_requests);
            let url =
                format!("https://api.github.com/{path}/{owner}/settings/billing/{report}/usage");
            let mut query = vec![
                ("year", day.year().to_string()),
                ("month", day.month().to_string()),
            ];
            if !full_month {
                query.push(("day", day.day().to_string()));
            }
            let page = get(
                agent,
                &url,
                &query,
                &[
                    ("Authorization", &format!("Bearer {key}")),
                    ("Accept", "application/vnd.github+json"),
                    ("X-GitHub-Api-Version", "2026-03-10"),
                ],
            )?;
            for item in array(&page["usageItems"])? {
                if !item["product"]
                    .as_str()
                    .is_some_and(|p| p.to_ascii_lowercase().starts_with("copilot"))
                {
                    continue;
                }
                rows.push(self.row(
                    &Value::Null,
                    item["model"].as_str(),
                    "USD",
                    money(&item["netAmount"], 1_000_000.0)?,
                )?);
            }
        }
        Ok(rows)
    }

    fn xai(
        &self,
        agent: &ureq::Agent,
        key: &str,
        period: Period,
        team: &str,
    ) -> Result<Vec<BilledRow>, String> {
        if !identifier(team) {
            return Err("Invalid xAI billing team".into());
        }
        let (start, end) = period.dates();
        let data = read_response(
            agent
                .post(format!(
                    "https://management-api.x.ai/v1/billing/teams/{team}/usage"
                ))
                .header("Authorization", format!("Bearer {key}"))
                .send_json(json!({
                    "analyticsRequest": {
                        "timeRange": {
                            "startTime": format!("{start} 00:00:00"),
                            "endTime": format!("{end} 00:00:00"),
                            "timezone": "Etc/GMT"
                        },
                        "timeUnit": "TIME_UNIT_DAY",
                        "values": [{"name": "usd", "aggregation": "AGGREGATION_SUM"}],
                        "groupBy": ["description"],
                        "filters": []
                    }
                })),
        )?;
        if data["limitReached"] == true {
            return Err("xAI billing results were truncated. Select a shorter period.".into());
        }
        array(&data["timeSeries"])?
            .iter()
            .map(|series| {
                let model = series["group"][0].as_str().and_then(|description| {
                    self.model_names
                        .get(description)
                        .map(String::as_str)
                        .or_else(|| {
                            let resource = description.strip_prefix("Chat ").unwrap_or(description);
                            resource.starts_with("grok-").then_some(resource)
                        })
                });
                let micros =
                    array(&series["dataPoints"])?
                        .iter()
                        .try_fold(0_i64, |total, point| {
                            total
                                .checked_add(money(&point["values"][0], 1_000_000.0)?)
                                .ok_or_else(|| "Billed amount outside supported range".to_string())
                        })?;
                self.row(&Value::Null, model, "USD", micros)
            })
            .collect()
    }

    fn google(
        &self,
        agent: &ureq::Agent,
        token: &str,
        project: &str,
        location: &str,
        query: &Value,
    ) -> Result<Vec<BilledRow>, String> {
        let bearer = format!("Bearer {token}");
        let root = format!("https://bigquery.googleapis.com/bigquery/v2/projects/{project}");
        let mut response = read_response(
            agent
                .post(format!("{root}/queries"))
                .header("Authorization", &bearer)
                .header("X-Goog-User-Project", project)
                .send_json(query),
        )?;
        let mut rows = Vec::new();
        for _ in 0..MAX_PAGES {
            let complete = response["jobComplete"]
                .as_bool()
                .ok_or("Missing BigQuery job status")?;
            if complete {
                for row in response["rows"].as_array().into_iter().flatten() {
                    let fields = array(&row["f"])?;
                    if fields.len() != 4 {
                        return Err("Invalid BigQuery billing row".into());
                    }
                    let model = fields[1]["v"]
                        .as_str()
                        .and_then(|sku| self.model_names.get(sku))
                        .map(String::as_str);
                    rows.push(
                        self.row(
                            &fields[0]["v"],
                            model,
                            fields[2]["v"]
                                .as_str()
                                .ok_or("Missing Google Cloud currency")?,
                            fields[3]["v"]
                                .as_str()
                                .and_then(|s| s.parse::<i64>().ok())
                                .ok_or("Invalid Google Cloud charge")?,
                        )?,
                    );
                }
            }
            let page_token = response["pageToken"].as_str().filter(|s| !s.is_empty());
            if complete && page_token.is_none() {
                return Ok(rows);
            }
            let job = response["jobReference"]["jobId"]
                .as_str()
                .filter(|s| identifier(s))
                .ok_or("Missing BigQuery job ID")?;
            let mut params = vec![
                ("location", location.into()),
                ("maxResults", "10000".into()),
                ("timeoutMs", "5000".into()),
            ];
            if let Some(page) = page_token {
                params.push(("pageToken", page.into()));
            }
            response = get(
                agent,
                &format!("{root}/queries/{job}"),
                &params,
                &[("Authorization", &bearer), ("X-Goog-User-Project", project)],
            )?;
        }
        Err("BigQuery billing query did not finish within the page limit".into())
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

fn github_report(day: NaiveDate, legacy_premium_requests: bool) -> &'static str {
    if legacy_premium_requests || (day.year(), day.month()) < (2026, 6) {
        "premium_request"
    } else {
        "ai_credit"
    }
}

fn google_query(
    period: Period,
    table: &str,
    location: &str,
    service_ids: &[String],
    sku_ids: &[String],
    max_bytes: u64,
) -> Result<Value, String> {
    if table.split('.').count() != 3
        || !table.split('.').all(identifier)
        || !identifier(location)
        || service_ids.is_empty()
        || service_ids.iter().any(|s| !identifier(s))
        || sku_ids.iter().any(|s| !identifier(s))
        || max_bytes == 0
    {
        return Err("Invalid BigQuery export table, service filter or byte limit".into());
    }
    let (start, end) = period.dates();
    let scalar = |name: &str, value: String| {
        json!({
            "name":name, "parameterType":{"type":"TIMESTAMP"}, "parameterValue":{"value":value}
        })
    };
    let strings = |name: &str, ids: &[String]| {
        json!({
            "name":name, "parameterType":{"type":"ARRAY","arrayType":{"type":"STRING"}},
            "parameterValue":{"arrayValues": ids.iter().map(|id| json!({"value":id})).collect::<Vec<_>>()}
        })
    };
    // All values are bound parameters except the validated BigQuery table identifier.
    let sql = format!(
        r#"
SELECT project.id, sku.id, currency,
       CAST(ROUND(SUM(CAST(cost AS NUMERIC) +
           IFNULL((SELECT SUM(CAST(credit.amount AS NUMERIC)) FROM UNNEST(credits) AS credit), 0))
           * 1000000) AS INT64)
FROM `{table}`
WHERE usage_start_time >= @start AND usage_start_time < @end
  AND service.id IN UNNEST(@services)
  AND (ARRAY_LENGTH(@skus) = 0 OR sku.id IN UNNEST(@skus))
GROUP BY 1, 2, 3"#
    );
    Ok(json!({
        "query":sql, "useLegacySql":false, "location":location, "parameterMode":"NAMED",
        "maximumBytesBilled":max_bytes.to_string(), "timeoutMs":10000, "maxResults":10000,
        "queryParameters":[scalar("start", format!("{start} 00:00:00+00")), scalar("end", format!("{end} 00:00:00+00")),
            strings("services",service_ids), strings("skus",sku_ids)]
    }))
}

fn github_periods(period: Period) -> Vec<(NaiveDate, bool)> {
    let (mut date, end) = period.dates();
    let mut result = Vec::new();
    while date < end {
        let next_month = date
            .with_day(1)
            .unwrap()
            .checked_add_months(chrono::Months::new(1))
            .unwrap();
        let full_month = date.day() == 1 && next_month <= end;
        result.push((date, full_month));
        date = if full_month {
            next_month
        } else {
            date.checked_add_days(Days::new(1)).unwrap()
        };
    }
    result
}

fn get(
    agent: &ureq::Agent,
    url: &str,
    query: &[(&str, String)],
    headers: &[(&str, &str)],
) -> Result<Value, String> {
    let mut req = agent.get(url).header("User-Agent", "scrooge-widget");
    for (name, value) in query {
        req = req.query(*name, value);
    }
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    read_response(req.call())
}

fn read_response(
    response: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<Value, String> {
    match response {
        Ok(mut res) if res.status().is_success() => res
            .body_mut()
            .read_json()
            .map_err(|_| "Invalid provider response".into()),
        Ok(res) => Err(format!("Provider returned HTTP {}", res.status().as_u16())),
        Err(ureq::Error::StatusCode(code)) => Err(format!("Provider returned HTTP {code}")),
        Err(_) => Err("Billing service could not be reached".into()),
    }
}

fn array(value: &Value) -> Result<&Vec<Value>, String> {
    value
        .as_array()
        .ok_or_else(|| "Invalid provider billing results".into())
}

fn next_page(page: &Value) -> Result<Option<String>, String> {
    if page["has_more"] == true {
        page["next_page"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| Some(s.into()))
            .ok_or_else(|| "Missing billing page cursor".into())
    } else {
        Ok(None)
    }
}

fn replace_page(query: &mut Vec<(&str, String)>, name: &'static str, page: String) {
    query.retain(|(k, _)| *k != name);
    query.push((name, page));
}

fn money(value: &Value, micros_per_unit: f64) -> Result<i64, String> {
    let amount = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())
        .ok_or("Invalid billed amount")?
        * micros_per_unit;
    if !amount.is_finite() || amount.abs() >= i64::MAX as f64 {
        return Err("Billed amount outside supported range".into());
    }
    Ok(amount.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_uses_months_only_when_the_full_month_is_selected() {
        let ms = |date: &str| {
            NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_millis() as u64
        };
        let period = Period::new(ms("2026-08-30"), ms("2026-10-02")).unwrap();
        let periods = github_periods(period);
        assert_eq!(periods.len(), 4);
        assert_eq!(periods[2], (Period::date(ms("2026-09-01")), true));
        assert!(!periods[3].1);
    }

    #[test]
    fn github_selects_its_billing_model_for_the_requested_date() {
        let may = NaiveDate::from_ymd_opt(2026, 5, 31).unwrap();
        let june = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        assert_eq!(github_report(may, false), "premium_request");
        assert_eq!(github_report(june, false), "ai_credit");
        assert_eq!(github_report(june, true), "premium_request");
    }

    #[test]
    fn google_limits_query_bytes_and_binds_service_ids() {
        let period = Period::new(0, DAY_MS).unwrap();
        let query = google_query(
            period,
            "billing.dataset.export",
            "EU",
            &["a-b".into()],
            &[],
            10_000_000,
        )
        .unwrap();
        assert_eq!(query["maximumBytesBilled"], "10000000");
        assert!(query["query"].as_str().unwrap().contains("@services"));
        assert!(!query["query"].as_str().unwrap().contains("'a-b'"));
        assert!(google_query(period, "p.d.t`;DELETE", "EU", &["a-b".into()], &[], 1).is_err());
    }

    #[test]
    fn billing_decimal_units_are_distinct() {
        assert_eq!(money(&json!("12.3456"), 10_000.0).unwrap(), 123456);
        assert_eq!(money(&json!(-0.0015), 1_000_000.0).unwrap(), -1500);
        assert!(money(&json!("NaN"), 1_000_000.0).is_err());
    }
}
