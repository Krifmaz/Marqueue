//! A site searched through its YAML definition.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, Utc};
use regex::Regex;
use reqwest::{Method, StatusCode};
use scoutline_core::backend::{Backend, BoxFuture, Download};
use scoutline_core::feed::FeedItem;
use scoutline_core::settings::SettingValue;
use scoutline_core::{IndexerError, SearchQuery, SearchType, category};
use scoutline_net::{Client, Jar, NetSettings, Route, build_session_client};
use scraper::Html;
use url::Url;

use crate::extract::{self, RowValues};
use crate::filters;
use crate::model::{Definition, Login, SearchPath};
use crate::template::{self, Value};

/// Where a request went and what came back.
struct Page {
    url: Url,
    status: StatusCode,
    content_type: String,
    bytes: Vec<u8>,
    /// Every URL a redirect pointed at, in order.
    redirects: Vec<Url>,
}

#[derive(Default)]
struct Session {
    logged_in: bool,
    last_request: Option<Instant>,
}

pub struct DefinitionIndexer {
    def: Arc<Definition>,
    base: Url,
    config: Value,
    /// (tracker category id, standard category id)
    categories: Vec<(String, u32)>,
    client: Client,
    jar: Arc<Jar>,
    session: tokio::sync::Mutex<Session>,
}

impl DefinitionIndexer {
    /// `base_url` overrides the definition's first link (mirrors);
    /// `settings` fills in the definition's settings (username, ...).
    pub fn new(
        def: Arc<Definition>,
        base_url: Option<&str>,
        settings: &std::collections::BTreeMap<String, SettingValue>,
        net: &NetSettings,
        route: &Route,
    ) -> Result<Self, String> {
        let base = base_url.unwrap_or(&def.links[0]);
        let mut base = Url::parse(base).map_err(|e| format!("bad site url {base:?}: {e}"))?;
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }

        let mut config: HashMap<String, Value> = HashMap::new();
        for s in &def.settings {
            let v = match (s.kind.as_str(), settings.get(&s.name)) {
                (
                    "info" | "info_cookie" | "info_flaresolverr" | "info_useragent"
                    | "info_category_8000",
                    _,
                ) => continue,
                ("checkbox", Some(SettingValue::Bool(b))) => Value::Bool(*b),
                ("checkbox", Some(other)) => {
                    Value::Bool(matches!(other.as_text().as_str(), "true" | "1"))
                }
                ("checkbox", None) => Value::Bool(s.default.as_deref() == Some("true")),
                (_, Some(v)) => Value::from(v.as_text()),
                ("select", None) => Value::from(
                    s.default
                        .clone()
                        .or_else(|| s.options.keys().next().cloned())
                        .unwrap_or_default(),
                ),
                (_, None) => Value::from(s.default.clone().unwrap_or_default()),
            };
            config.insert(s.name.clone(), v);
        }
        // Settings not declared by the definition still reach templates.
        for (k, v) in settings {
            config.entry(k.clone()).or_insert_with(|| match v {
                SettingValue::Bool(b) => Value::Bool(*b),
                other => Value::from(other.as_text()),
            });
        }
        if let Some(login) = &def.login {
            if login.method == "cookie" && !config.get("cookie").is_some_and(Value::truthy) {
                return Err(format!("{}: needs the `cookie` setting", def.id));
            }
        }

        let categories = def
            .category_map()
            .into_iter()
            .filter_map(|(tid, name)| Some((tid, category_id(&name)?)))
            .collect();

        let jar = Arc::new(Jar::default());
        let client =
            build_session_client(net, route, Arc::clone(&jar)).map_err(|e| e.to_string())?;
        Ok(Self {
            def,
            base,
            config: Value::Map(config),
            categories,
            client,
            jar,
            session: Default::default(),
        })
    }

    pub fn definition(&self) -> &Definition {
        &self.def
    }

    /// Template data for one search.
    fn vars(&self, q: &SearchQuery) -> Value {
        let mut keywords = q.term.clone().unwrap_or_default();
        if q.search_type == SearchType::Tv {
            if let Some(s) = q.season {
                keywords.push_str(&format!(" S{s:02}"));
                if let Some(e) = q.episode.as_deref().and_then(|e| e.parse::<u32>().ok()) {
                    keywords.push_str(&format!("E{e:02}"));
                }
            }
        }
        let keywords = filters::apply(
            keywords.trim(),
            &self.def.search.keywordsfilters,
            Utc::now(),
        )
        .unwrap_or_else(|_| keywords.trim().to_owned());
        let imdb = q
            .imdb_id
            .as_deref()
            .map(|i| format!("tt{:0>7}", i.trim_start_matches("tt")));
        let num = |n: Option<u32>| n.map(|n| n.to_string());
        let query = Value::from_iter([
            ("Type", Value::from(type_name(q.search_type))),
            ("Q", Value::from(keywords.clone())),
            ("Keywords", Value::from(keywords.clone())),
            ("Season", Value::from(num(q.season))),
            ("Ep", Value::from(q.episode.clone())),
            ("IMDBID", Value::from(imdb.clone())),
            (
                "IMDBIDShort",
                Value::from(imdb.map(|i| i.trim_start_matches("tt").to_owned())),
            ),
            ("TMDBID", Value::from(num(q.tmdb_id))),
            ("TVDBID", Value::from(num(q.tvdb_id))),
            ("Author", Value::from(q.author.clone())),
            ("Title", Value::from(q.title.clone())),
            ("Artist", Value::from(q.artist.clone())),
            ("Album", Value::from(q.album.clone())),
            ("Limit", Value::from(num(q.limit))),
            ("Offset", Value::from(num(q.offset))),
        ]);
        let cats = self.tracker_categories(&q.categories);
        Value::from_iter([
            ("Keywords", Value::from(keywords)),
            ("Query", query),
            ("Config", self.config.clone()),
            (
                "Categories",
                Value::List(cats.into_iter().map(Value::from).collect()),
            ),
            (
                "Today",
                Value::from_iter([("Year", Value::from(Utc::now().year().to_string()))]),
            ),
            ("True", Value::Bool(true)),
            ("False", Value::Bool(false)),
        ])
    }

    /// Tracker ids for the requested standard categories (parents include
    /// their children).
    fn tracker_categories(&self, wanted: &[u32]) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (tid, nid) in &self.categories {
            let hit = wanted.contains(nid)
                || category::parent_id(*nid).is_some_and(|p| wanted.contains(&p));
            if hit && !out.contains(tid) {
                out.push(tid.clone());
            }
        }
        out
    }

    fn standard_categories(&self, value: &str) -> Vec<u32> {
        let mut out = Vec::new();
        for tid in value.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            for (t, nid) in &self.categories {
                if t == tid {
                    for id in [category::parent_id(*nid), Some(*nid)]
                        .into_iter()
                        .flatten()
                    {
                        if !out.contains(&id) {
                            out.push(id);
                        }
                    }
                }
            }
        }
        out
    }

    fn resolve(&self, link: &str) -> Option<String> {
        let link = link.trim();
        if link.is_empty() {
            return None;
        }
        if link.starts_with("magnet:") {
            return Some(link.to_owned());
        }
        self.base.join(link).ok().map(|u| u.to_string())
    }

    /// One HTTP request, following up to 5 redirects by hand so cookies
    /// set along the way land in the jar and login redirects are visible.
    async fn request(
        &self,
        method: Method,
        url: Url,
        form: Option<Vec<(String, String)>>,
        headers: &[(String, String)],
    ) -> Result<Page, IndexerError> {
        self.pace().await;
        let http = |e: reqwest::Error| IndexerError::Http(e.without_url().to_string());
        let mut redirects = Vec::new();
        let (mut method, mut url, mut form) = (method, url, form);
        loop {
            let mut req = self.client.request(method.clone(), url.clone());
            for (k, v) in headers {
                req = req.header(k, v);
            }
            if let Some(f) = &form {
                req = req.form(f);
            }
            let resp = req.send().await.map_err(http)?;
            let status = resp.status();
            if status.is_redirection() && redirects.len() < 5 {
                if let Some(next) = resp
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                {
                    redirects.push(next.clone());
                    url = next;
                    // 301/302/303 after a POST become a GET, as browsers do.
                    if status != StatusCode::TEMPORARY_REDIRECT
                        && status != StatusCode::PERMANENT_REDIRECT
                    {
                        method = Method::GET;
                        form = None;
                    }
                    continue;
                }
            }
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let final_url = resp.url().clone();
            let bytes = resp.bytes().await.map_err(http)?.to_vec();
            return Ok(Page {
                url: final_url,
                status,
                content_type,
                bytes,
                redirects,
            });
        }
    }

    /// Waits out the definition's `requestDelay` between requests.
    async fn pace(&self) {
        let delay = Duration::from_secs_f64(self.def.request_delay.unwrap_or(0.0).max(0.0));
        let wait = {
            let mut s = self.session.lock().await;
            let wait = s
                .last_request
                .map_or(Duration::ZERO, |t| delay.saturating_sub(t.elapsed()));
            s.last_request = Some(Instant::now() + wait);
            wait
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    fn decode(&self, page: &Page) -> String {
        let label = self
            .def
            .encoding
            .clone()
            .or_else(|| {
                page.content_type
                    .split("charset=")
                    .nth(1)
                    .map(|c| c.trim_matches(['"', ' ', ';']).to_owned())
            })
            .unwrap_or_else(|| "utf-8".into());
        let enc = encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::UTF_8);
        enc.decode(&page.bytes).0.into_owned()
    }

    fn render(&self, t: &str, vars: &Value) -> Result<String, IndexerError> {
        template::render(t, vars).map_err(|e| IndexerError::Parse(e.to_string()))
    }

    fn render_headers(
        &self,
        headers: &indexmap::IndexMap<String, crate::model::StringOrList>,
        vars: &Value,
    ) -> Result<Vec<(String, String)>, IndexerError> {
        let mut out = Vec::new();
        for (k, v) in headers {
            let values = match v {
                crate::model::StringOrList::One(s) => vec![s.clone()],
                crate::model::StringOrList::Many(l) => l.clone(),
            };
            for v in values {
                out.push((k.clone(), self.render(&v, vars)?));
            }
        }
        Ok(out)
    }

    // ---- login --------------------------------------------------------------

    async fn ensure_login(&self, force: bool) -> Result<(), IndexerError> {
        let Some(login) = &self.def.login else {
            return Ok(());
        };
        if !force && self.session.lock().await.logged_in {
            return Ok(());
        }
        self.login(login).await?;
        self.session.lock().await.logged_in = true;
        Ok(())
    }

    async fn login(&self, login: &Login) -> Result<(), IndexerError> {
        let fail = |m: String| IndexerError::Api {
            code: 100,
            description: m,
        };
        let vars = self.vars(&SearchQuery::default());
        let headers = self.render_headers(&login.headers, &vars)?;
        let path_url = |p: &str| -> Result<Url, IndexerError> {
            let p = self.render(p, &vars)?;
            self.base
                .join(&p)
                .map_err(|e| fail(format!("bad login path: {e}")))
        };
        let mut inputs: Vec<(String, String)> = Vec::new();
        for (k, v) in &login.inputs {
            inputs.push((k.clone(), self.render(v, &vars)?));
        }

        let page = match login.method.as_str() {
            "cookie" => {
                let cookie = self.config_text("cookie");
                for c in cookie.split(';').map(str::trim).filter(|c| !c.is_empty()) {
                    self.jar.add_cookie_str(c, &self.base);
                }
                None
            }
            "post" => {
                let url = path_url(login.path.as_deref().unwrap_or(""))?;
                Some(
                    self.request(Method::POST, url, Some(inputs), &headers)
                        .await?,
                )
            }
            "get" | "oneurl" => {
                let mut url = path_url(login.path.as_deref().unwrap_or(""))?;
                url.query_pairs_mut().extend_pairs(&inputs);
                Some(self.request(Method::GET, url, None, &headers).await?)
            }
            "form" => {
                let url = path_url(login.path.as_deref().unwrap_or(""))?;
                let form_page = self
                    .request(Method::GET, url.clone(), None, &headers)
                    .await?;
                let body = self.decode(&form_page);
                let (action, method, mut fields) = read_form(&body, login).map_err(fail)?;
                for (k, v) in inputs {
                    set_field(&mut fields, &k, v);
                }
                let target = match &login.submitpath {
                    Some(p) => path_url(p)?,
                    None => form_page
                        .url
                        .join(&action)
                        .map_err(|e| fail(e.to_string()))?,
                };
                let page = if method.eq_ignore_ascii_case("get") {
                    let mut t = target;
                    t.query_pairs_mut().extend_pairs(&fields);
                    self.request(Method::GET, t, None, &headers).await?
                } else {
                    self.request(Method::POST, target, Some(fields), &headers)
                        .await?
                };
                Some(page)
            }
            other => return Err(fail(format!("login method {other:?} is not supported"))),
        };

        if let Some(page) = &page {
            let body = self.decode(page);
            if let Some(msg) = self.page_error(&login.error, &body) {
                return Err(fail(format!("login failed: {msg}")));
            }
        }
        if let Some(test) = &login.test {
            let url = path_url(test.path.as_deref().unwrap_or(""))?;
            let page = self.request(Method::GET, url, None, &headers).await?;
            let landed_on_login = login.path.as_deref().is_some_and(|p| {
                page.redirects
                    .iter()
                    .any(|u| u.path().ends_with(p.trim_start_matches('/')))
            });
            let body = self.decode(&page);
            let ok = match &test.selector {
                Some(sel) => {
                    let doc = Html::parse_document(&body);
                    let root = doc.root_element();
                    extract::select_first(root, root, sel)
                        .map_err(fail)?
                        .is_some()
                }
                None => page.status.is_success(),
            };
            if !ok || landed_on_login {
                return Err(fail(
                    "login failed: still logged out (check username/password or cookie)".into(),
                ));
            }
        }
        Ok(())
    }

    fn config_text(&self, name: &str) -> String {
        match &self.config {
            Value::Map(m) => m.get(name).map(Value::render).unwrap_or_default(),
            _ => String::new(),
        }
    }

    /// The first error block that matches the page, as a message.
    fn page_error(&self, blocks: &[crate::model::ErrorBlock], body: &str) -> Option<String> {
        if blocks.is_empty() {
            return None;
        }
        let doc = Html::parse_document(body);
        let root = doc.root_element();
        for b in blocks {
            if let Ok(Some(el)) = extract::select_first(root, root, &b.selector) {
                let text = match &b.message {
                    Some(m) => match (&m.selector, &m.text) {
                        (_, Some(t)) => t.clone(),
                        (Some(s), _) => extract::select_first(root, root, s)
                            .ok()
                            .flatten()
                            .map(|e| e.text().collect::<String>())
                            .unwrap_or_default(),
                        _ => String::new(),
                    },
                    None => el.text().collect::<String>(),
                };
                let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
                return Some(if text.is_empty() {
                    "the site reported an error".into()
                } else {
                    text
                });
            }
        }
        None
    }

    // ---- search -------------------------------------------------------------

    async fn search_path(
        &self,
        path: &SearchPath,
        vars: &Value,
    ) -> Result<(Page, String), IndexerError> {
        let s = &self.def.search;
        let mut inputs: Vec<(String, String)> = Vec::new();
        let mut raw = String::new();
        let all = if path.inheritinputs {
            s.inputs
                .iter()
                .chain(path.inputs.iter())
                .collect::<Vec<_>>()
        } else {
            path.inputs.iter().collect()
        };
        for (k, v) in all {
            let v = self.render(v, vars)?;
            if k == "$raw" {
                raw.push_str(&v);
            } else {
                inputs.push((k.clone(), v));
            }
        }
        let rendered = self.render(&path.path, vars)?;
        let mut url = self
            .base
            .join(&rendered)
            .map_err(|e| IndexerError::Parse(format!("bad search path {rendered:?}: {e}")))?;
        let headers = self.render_headers(&s.headers, vars)?;
        let method = path.method.as_deref().unwrap_or("get");
        let page = if method.eq_ignore_ascii_case("post") {
            self.request(Method::POST, url, Some(inputs), &headers)
                .await?
        } else {
            if !inputs.is_empty() {
                url.query_pairs_mut().extend_pairs(&inputs);
            }
            if !raw.is_empty() {
                let q = match url.query() {
                    Some(q) if !q.is_empty() => format!("{q}&{}", raw.trim_start_matches('&')),
                    _ => raw.trim_start_matches('&').to_owned(),
                };
                url.set_query(Some(&q));
            }
            self.request(Method::GET, url, None, &headers).await?
        };
        let body = self.decode(&page);
        Ok((page, body))
    }

    fn logged_out(&self, page: &Page) -> bool {
        let Some(login) = &self.def.login else {
            return false;
        };
        let Some(path) = login.path.as_deref() else {
            return false;
        };
        let path = path.trim_start_matches('/');
        !path.is_empty()
            && page
                .redirects
                .iter()
                .chain(std::iter::once(&page.url))
                .any(|u| {
                    u.path()
                        .trim_start_matches('/')
                        .starts_with(path.split('?').next().unwrap_or(path))
                })
    }

    async fn run_search(&self, query: &SearchQuery) -> Result<Vec<FeedItem>, IndexerError> {
        self.ensure_login(false).await?;
        let vars = self.vars(query);
        let wanted = self.tracker_categories(&query.categories);
        let mut items = Vec::new();
        for path in self.def.search_paths() {
            if !path.categories.is_empty()
                && !wanted.is_empty()
                && !path.categories.iter().any(|c| wanted.contains(c))
            {
                continue;
            }
            let (mut page, mut body) = self.search_path(&path, &vars).await?;
            if self.logged_out(&page) {
                self.ensure_login(true).await?;
                (page, body) = self.search_path(&path, &vars).await?;
                if self.logged_out(&page) {
                    return Err(IndexerError::Api {
                        code: 100,
                        description: "logged out by the site".into(),
                    });
                }
            }
            if !page.status.is_success() {
                return Err(IndexerError::Status(page.status.as_u16()));
            }
            items.extend(self.parse_results(&body, path.response.as_ref())?);
        }
        Ok(items)
    }

    /// Turns one search response into feed items. Public so definitions
    /// can be checked against saved pages offline.
    pub fn parse_results(
        &self,
        body: &str,
        response: Option<&crate::model::ResponseType>,
    ) -> Result<Vec<FeedItem>, IndexerError> {
        let now = Utc::now();
        let s = &self.def.search;
        let vars = self.vars(&SearchQuery::default());
        let body =
            filters::apply(body, &s.preprocessingfilters, now).map_err(IndexerError::Parse)?;
        let json = response.and_then(|r| r.kind.as_deref()) == Some("json");
        if let Some(msg) = response.and_then(|r| r.no_results_message.as_deref()) {
            if !msg.is_empty() && body.trim() == msg {
                return Ok(Vec::new());
            }
        }
        let (rows, _errors) = if json {
            extract::json_rows(&body, &s.rows, &s.fields, &vars, now)
        } else {
            if let Some(msg) = self.page_error(&s.error, &body) {
                return Err(IndexerError::Api {
                    code: 900,
                    description: msg,
                });
            }
            extract::html_rows(&body, &s.rows, &s.fields, &vars, now)
        }
        .map_err(IndexerError::Parse)?;
        Ok(rows
            .into_iter()
            .filter_map(|r| self.to_item(r, now))
            .collect())
    }

    fn to_item(&self, v: RowValues, now: DateTime<Utc>) -> Option<FeedItem> {
        let get = |k: &str| v.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
        let title = get("title")?.to_owned();
        let details = get("details").and_then(|d| self.resolve(d));
        let mut magnet = get("magnet").and_then(|m| self.resolve(m));
        let mut download = get("download").and_then(|d| self.resolve(d));
        if download
            .as_deref()
            .is_some_and(|d| d.starts_with("magnet:"))
        {
            magnet = magnet.or(download.take());
        }
        let link = download.clone().or_else(|| magnet.clone())?;

        let mut attrs: Vec<(String, String)> = Vec::new();
        let push = |attrs: &mut Vec<(String, String)>, k: &str, val: String| {
            attrs.push((k.to_owned(), val))
        };
        let seeders = get("seeders").and_then(extract::parse_count);
        let leechers = get("leechers").and_then(extract::parse_count);
        if let Some(s) = seeders {
            push(&mut attrs, "seeders", s.to_string());
        }
        if seeders.is_some() || leechers.is_some() {
            push(
                &mut attrs,
                "peers",
                (seeders.unwrap_or(0) + leechers.unwrap_or(0)).to_string(),
            );
        }
        if let Some(g) = get("grabs").and_then(extract::parse_count) {
            push(&mut attrs, "grabs", g.to_string());
        }
        if let Some(f) = get("files").and_then(extract::parse_count) {
            push(&mut attrs, "files", f.to_string());
        }
        let hash = get("infohash")
            .map(str::to_owned)
            .or_else(|| magnet.as_deref().and_then(btih));
        if let Some(h) = hash {
            push(&mut attrs, "infohash", h);
        }
        if let Some(m) = &magnet {
            push(&mut attrs, "magneturl", m.clone());
        }
        for (field, attr) in [
            ("downloadvolumefactor", "downloadvolumefactor"),
            ("uploadvolumefactor", "uploadvolumefactor"),
            ("minimumratio", "minimumratio"),
            ("minimumseedtime", "minimumseedtime"),
            ("tmdbid", "tmdbid"),
            ("tvdbid", "tvdbid"),
            ("genre", "genre"),
        ] {
            if let Some(x) = get(field) {
                push(&mut attrs, attr, x.to_owned());
            }
        }
        if !attrs.iter().any(|(k, _)| k == "downloadvolumefactor") {
            push(&mut attrs, "downloadvolumefactor", "1".into());
        }
        if !attrs.iter().any(|(k, _)| k == "uploadvolumefactor") {
            push(&mut attrs, "uploadvolumefactor", "1".into());
        }
        if let Some(i) = get("imdbid").or_else(|| get("imdb")) {
            static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{5,})").unwrap());
            if let Some(c) = DIGITS.captures(i) {
                push(&mut attrs, "imdbid", format!("tt{:0>7}", &c[1]));
            }
        }

        let mut categories = get("category")
            .map(|c| self.standard_categories(c))
            .unwrap_or_default();
        if categories.is_empty() {
            if let Some(desc) = get("categorydesc") {
                for m in &self.def.caps.categorymappings {
                    if m.desc
                        .as_deref()
                        .is_some_and(|d| d.eq_ignore_ascii_case(desc))
                    {
                        categories.extend(self.standard_categories(&m.id));
                    }
                }
            }
        }
        for c in &categories {
            attrs.push(("category".into(), c.to_string()));
        }

        Some(FeedItem {
            title: filters::html_decode(&title),
            guid: details.clone().or_else(|| Some(link.clone())),
            link: Some(link.clone()),
            comments: details,
            published: Some(
                get("date")
                    .and_then(|d| crate::dates::parse_lenient(d, now))
                    .unwrap_or(now),
            ),
            size: get("size").and_then(extract::parse_size),
            categories,
            enclosure_url: download.or(magnet),
            attrs,
        })
    }

    async fn fetch_download(&self, url: &str) -> Result<Download, IndexerError> {
        self.ensure_login(false).await?;
        let mut target = Url::parse(url).map_err(|e| IndexerError::Parse(e.to_string()))?;
        if target.host_str() != self.base.host_str()
            && !self.def.links.iter().chain(&self.def.legacylinks).any(|l| {
                Url::parse(l)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned))
                    == target.host_str().map(str::to_owned)
            })
        {
            return Err(IndexerError::Http(
                "download link is not on this indexer's site".into(),
            ));
        }
        // Some sites link to a details page; the definition says where the
        // real download link is on it.
        if let Some(dl) = self
            .def
            .download
            .as_ref()
            .filter(|d| !d.selectors.is_empty())
        {
            let page = self.request(Method::GET, target.clone(), None, &[]).await?;
            let body = self.decode(&page);
            target = find_download_link(&body, dl, &page.url)?.ok_or_else(|| {
                IndexerError::Parse("download link not found on the details page".into())
            })?;
        }
        let page = self.request(Method::GET, target, None, &[]).await?;
        if !page.status.is_success() {
            return Err(IndexerError::Status(page.status.as_u16()));
        }
        if self.logged_out(&page) || page.content_type.starts_with("text/html") {
            return Err(IndexerError::Api {
                code: 100,
                description: "the site returned a web page instead of a file (logged out?)".into(),
            });
        }
        Ok(Download {
            content_type: if page.content_type.is_empty() {
                "application/x-bittorrent".into()
            } else {
                page.content_type
            },
            bytes: page.bytes,
        })
    }
}

impl Backend for DefinitionIndexer {
    fn search<'a>(
        &'a self,
        query: &'a SearchQuery,
    ) -> BoxFuture<'a, Result<Vec<FeedItem>, IndexerError>> {
        Box::pin(self.run_search(query))
    }

    fn needs_download_proxy(&self) -> bool {
        self.def.login.is_some()
            || self
                .def
                .download
                .as_ref()
                .is_some_and(|d| !d.selectors.is_empty())
    }

    fn download<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Download, IndexerError>> {
        Box::pin(self.fetch_download(url))
    }
}

fn type_name(t: SearchType) -> &'static str {
    match t {
        SearchType::General => "search",
        SearchType::Tv => "tvsearch",
        SearchType::Movie => "movie",
        SearchType::Book => "book",
        SearchType::Music => "music",
    }
}

/// Standard category id for a name like `Movies/HD` or `TV`.
pub fn category_id(name: &str) -> Option<u32> {
    let n = name.trim();
    category::ALL
        .iter()
        .find(|c| c.name.eq_ignore_ascii_case(n))
        .map(|c| c.id)
}

fn btih(magnet: &str) -> Option<String> {
    static HASH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)btih:([a-f0-9]{40})").unwrap());
    HASH.captures(magnet).map(|c| c[1].to_lowercase())
}

/// The real download link on a details page.
fn find_download_link(
    body: &str,
    dl: &crate::model::Download,
    page_url: &Url,
) -> Result<Option<Url>, IndexerError> {
    let doc = Html::parse_document(body);
    let root = doc.root_element();
    for s in &dl.selectors {
        if let Ok(Some(el)) = extract::select_first(root, root, &s.selector) {
            let raw = match &s.attribute {
                Some(a) => el.value().attr(a).unwrap_or("").to_owned(),
                None => el.text().collect(),
            };
            let v = filters::apply(&raw, &s.filters, Utc::now()).map_err(IndexerError::Parse)?;
            if let Ok(u) = page_url.join(v.trim()) {
                return Ok(Some(u));
            }
        }
    }
    Ok(None)
}

fn set_field(fields: &mut Vec<(String, String)>, name: &str, value: String) {
    match fields.iter_mut().find(|(k, _)| k == name) {
        Some(f) => f.1 = value,
        None => fields.push((name.to_owned(), value)),
    }
}

/// Action, method and fields of a login form.
type LoginForm = (String, String, Vec<(String, String)>);

/// A login form's action, method and pre-filled fields, plus the
/// definition's selector inputs (CSRF tokens and the like).
fn read_form(body: &str, login: &Login) -> Result<LoginForm, String> {
    let selector = login.form.as_deref().unwrap_or("form");
    let doc = Html::parse_document(body);
    let root = doc.root_element();
    let form = extract::select_first(root, root, selector)?
        .ok_or_else(|| format!("login form {selector:?} not found"))?;
    let action = form.value().attr("action").unwrap_or("").to_owned();
    let method = form.value().attr("method").unwrap_or("post").to_owned();
    let mut fields = Vec::new();
    let inputs = scraper::Selector::parse("input[name], select[name], textarea[name]").unwrap();
    for el in form.select(&inputs) {
        let name = el.value().attr("name").unwrap_or("").to_owned();
        let kind = el
            .value()
            .attr("type")
            .unwrap_or("text")
            .to_ascii_lowercase();
        if matches!(kind.as_str(), "submit" | "button" | "image" | "file") {
            continue;
        }
        if matches!(kind.as_str(), "checkbox" | "radio") && el.value().attr("checked").is_none() {
            continue;
        }
        let value = el.value().attr("value").unwrap_or("").to_owned();
        fields.push((name, value));
    }
    for (name, si) in &login.selectorinputs {
        match extract::select_first(root, root, &si.selector)? {
            Some(el) => {
                let v = match &si.attribute {
                    Some(a) => el.value().attr(a).unwrap_or("").to_owned(),
                    None => el.text().collect::<String>().trim().to_owned(),
                };
                set_field(&mut fields, name, v);
            }
            None if si.optional => {}
            None => return Err(format!("login form has no {:?}", si.selector)),
        }
    }
    Ok((action, method, fields))
}
