//! The Cardigann-style YAML indexer definition, as used by Prowlarr and
//! Jackett. Only what Scoutline reads is modelled; unknown keys are kept
//! out of the way so newer upstream definitions still load.

use indexmap::IndexMap;
use serde::Deserialize;

/// One `<id>.yml` file.
#[derive(Debug, Clone, Deserialize)]
pub struct Definition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub language: String,
    /// `public`, `private` or `semi-private`.
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub encoding: Option<String>,
    /// Seconds to wait between requests to this site.
    #[serde(default, rename = "requestDelay")]
    pub request_delay: Option<f64>,
    pub links: Vec<String>,
    #[serde(default)]
    pub legacylinks: Vec<String>,
    #[serde(default)]
    pub followredirect: bool,
    #[serde(default)]
    pub caps: Caps,
    #[serde(default)]
    pub settings: Vec<Setting>,
    #[serde(default)]
    pub login: Option<Login>,
    pub search: Search,
    #[serde(default)]
    pub download: Option<Download>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Caps {
    /// Short form: tracker category id -> standard category name.
    #[serde(default)]
    pub categories: IndexMap<String, String>,
    #[serde(default)]
    pub categorymappings: Vec<CategoryMapping>,
    /// Search mode -> supported parameters, e.g. `tv-search: [q, season, ep]`.
    #[serde(default)]
    pub modes: IndexMap<String, Vec<String>>,
    #[serde(default)]
    pub allowrawsearch: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CategoryMapping {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    /// Standard category name, e.g. `Movies/HD`. Missing for site-only ones.
    #[serde(default)]
    pub cat: Option<String>,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub default: bool,
}

/// A user setting, e.g. username, password or a sort order.
#[derive(Debug, Clone, Deserialize)]
pub struct Setting {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "opt_scalar")]
    pub default: Option<String>,
    #[serde(default)]
    pub options: IndexMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Login {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub submitpath: Option<String>,
    /// `post`, `form`, `cookie`, `get` or `oneurl`.
    #[serde(default = "default_post")]
    pub method: String,
    /// CSS selector of the login form (method `form`).
    #[serde(default)]
    pub form: Option<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, String>,
    #[serde(default)]
    pub selectorinputs: IndexMap<String, SelectorInput>,
    #[serde(default)]
    pub getselectorinputs: IndexMap<String, SelectorInput>,
    #[serde(default)]
    pub error: Vec<ErrorBlock>,
    #[serde(default)]
    pub test: Option<PageTest>,
    #[serde(default)]
    pub headers: IndexMap<String, StringOrList>,
}

fn default_post() -> String {
    "post".to_owned()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SelectorInput {
    pub selector: String,
    #[serde(default)]
    pub attribute: Option<String>,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ErrorBlock {
    #[serde(default)]
    pub path: Option<String>,
    pub selector: String,
    #[serde(default)]
    pub message: Option<Field>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageTest {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub selector: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Search {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub paths: Vec<SearchPath>,
    #[serde(default)]
    pub inputs: IndexMap<String, String>,
    #[serde(default)]
    pub headers: IndexMap<String, StringOrList>,
    #[serde(default)]
    pub keywordsfilters: Vec<Filter>,
    #[serde(default)]
    pub preprocessingfilters: Vec<Filter>,
    #[serde(default)]
    pub error: Vec<ErrorBlock>,
    pub rows: Rows,
    pub fields: IndexMap<String, Field>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchPath {
    pub path: String,
    #[serde(default)]
    pub method: Option<String>,
    /// Only use this path when one of these tracker categories is searched.
    #[serde(default, deserialize_with = "strings")]
    pub categories: Vec<String>,
    #[serde(default)]
    pub inputs: IndexMap<String, String>,
    /// When false, the search-level inputs are not added.
    #[serde(default = "yes")]
    pub inheritinputs: bool,
    #[serde(default)]
    pub followredirect: bool,
    #[serde(default)]
    pub response: Option<ResponseType>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResponseType {
    /// `html` (default) or `json`.
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default, rename = "noResultsMessage")]
    pub no_results_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rows {
    pub selector: String,
    /// Skip this many rows after each match (row spans).
    #[serde(default)]
    pub after: usize,
    #[serde(default)]
    pub remove: Option<String>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default)]
    pub dateheaders: Option<Field>,
    /// JSON: expand this array inside each row.
    #[serde(default)]
    pub attribute: Option<String>,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default, rename = "missingAttributeEqualsNoResults")]
    pub missing_attribute_is_empty: bool,
    #[serde(default)]
    pub count: Option<Field>,
}

/// How one result field is read.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Field {
    #[serde(default)]
    pub selector: Option<String>,
    #[serde(default)]
    pub attribute: Option<String>,
    /// Elements to remove inside the match before reading its text.
    #[serde(default)]
    pub remove: Option<String>,
    /// A template instead of a selector.
    #[serde(default, deserialize_with = "opt_scalar")]
    pub text: Option<String>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    /// Selector -> value: the first selector that matches gives the value.
    #[serde(default, deserialize_with = "scalar_map")]
    pub case: IndexMap<String, String>,
    #[serde(default)]
    pub optional: bool,
    #[serde(default, deserialize_with = "opt_scalar")]
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Filter {
    pub name: String,
    #[serde(default)]
    pub args: FilterArgs,
}

/// Filter arguments: none, one scalar, or a list.
#[derive(Debug, Clone, Default)]
pub struct FilterArgs(pub Vec<String>);

impl FilterArgs {
    pub fn get(&self, i: usize) -> Option<&str> {
        self.0.get(i).map(String::as_str)
    }
}

impl<'de> Deserialize<'de> for FilterArgs {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Self(match Scalarish::deserialize(d)? {
            Scalarish::One(s) => vec![s.into_string()],
            Scalarish::Many(v) => v.into_iter().map(Scalar::into_string).collect(),
            Scalarish::None => Vec::new(),
        }))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Download {
    #[serde(default)]
    pub selectors: Vec<DownloadSelector>,
    #[serde(default)]
    pub method: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DownloadSelector {
    pub selector: String,
    #[serde(default)]
    pub attribute: Option<String>,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum StringOrList {
    One(String),
    Many(Vec<String>),
}

// ---- lenient scalars ------------------------------------------------------
// Definitions write ids, defaults and args as strings, numbers or booleans
// interchangeably.

#[derive(Deserialize)]
#[serde(untagged)]
enum Scalar {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Scalar {
    fn into_string(self) -> String {
        match self {
            Self::Str(s) => s,
            Self::Int(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::Bool(b) => b.to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Scalarish {
    One(Scalar),
    Many(Vec<Scalar>),
    None,
}

fn string_or_number<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Scalar::deserialize(d)?.into_string())
}

fn opt_scalar<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Scalar>::deserialize(d)?.map(Scalar::into_string))
}

fn strings<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(Vec::<Scalar>::deserialize(d)?
        .into_iter()
        .map(Scalar::into_string)
        .collect())
}

fn scalar_map<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<IndexMap<String, String>, D::Error> {
    Ok(IndexMap::<String, Scalar>::deserialize(d)?
        .into_iter()
        .map(|(k, v)| (k, v.into_string()))
        .collect())
}

impl Definition {
    pub fn from_yaml(text: &str) -> Result<Self, String> {
        let d: Self = serde_saphyr::from_str(text).map_err(|e| e.to_string())?;
        if d.links.is_empty() {
            return Err(format!("definition {}: no links", d.id));
        }
        if d.search.path.is_none() && d.search.paths.is_empty() {
            return Err(format!("definition {}: no search path", d.id));
        }
        Ok(d)
    }

    /// The search paths, with the single `path:` form folded in.
    pub fn search_paths(&self) -> Vec<SearchPath> {
        if !self.search.paths.is_empty() {
            return self.search.paths.clone();
        }
        vec![SearchPath {
            path: self.search.path.clone().unwrap_or_default(),
            method: None,
            categories: Vec::new(),
            inputs: IndexMap::new(),
            inheritinputs: true,
            followredirect: false,
            response: None,
        }]
    }

    /// Tracker category id -> standard category name, both forms merged.
    pub fn category_map(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .caps
            .categories
            .iter()
            .map(|(id, cat)| (id.clone(), cat.clone()))
            .collect();
        out.extend(
            self.caps
                .categorymappings
                .iter()
                .filter_map(|m| Some((m.id.clone(), m.cat.clone()?))),
        );
        out
    }
}
