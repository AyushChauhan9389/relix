//! Stremio addon protocol: install URLs, manifests, request routing, responses.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use url::Url;

/// JavaScript's encodeURIComponent, which is what Stremio uses for path segments.
const COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-').remove(b'_').remove(b'.').remove(b'!').remove(b'~')
    .remove(b'*').remove(b'\'').remove(b'(').remove(b')');
/// Ids keep ':' readable, e.g. `tt0903747:1:2`.
const ID: &AsciiSet = &COMPONENT.remove(b':');

#[derive(Debug, PartialEq)]
pub struct InvalidManifestUrl;

/// Normalizes user input or a `stremio://` deep link into an https manifest URL.
pub fn manifest_url(input: &str) -> Result<Url, InvalidManifestUrl> {
    let input = input.trim();
    let input = match input.strip_prefix("stremio://") {
        Some(rest) => format!("https://{rest}"),
        None => input.to_owned(),
    };
    let url = Url::parse(&input).map_err(|_| InvalidManifestUrl)?;
    let ok = matches!(url.scheme(), "https" | "http") && url.path().ends_with("/manifest.json");
    ok.then_some(url).ok_or(InvalidManifestUrl)
}

/// The addon's base URL: the manifest URL without `/manifest.json`.
pub fn base_url(manifest: &Url) -> &str {
    manifest.as_str().trim_end_matches("/manifest.json")
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub version: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub logo: Option<String>,
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub id_prefixes: Option<Vec<String>>,
    #[serde(default)]
    pub resources: Vec<Resource>,
    #[serde(default)]
    pub catalogs: Vec<Catalog>,
    #[serde(default)]
    pub behavior_hints: ManifestHints,
}

/// A resource is either a bare name (inherits the manifest's types and idPrefixes)
/// or an object with its own.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum Resource {
    Short(String),
    #[serde(rename_all = "camelCase")]
    Full {
        name: String,
        #[serde(default)]
        types: Option<Vec<String>>,
        #[serde(default)]
        id_prefixes: Option<Vec<String>>,
    },
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub r#type: String,
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub extra: Vec<ExtraProp>,
    /// Legacy form, still sent by some addons alongside or instead of `extra`.
    #[serde(default)]
    pub extra_required: Vec<String>,
    #[serde(default)]
    pub extra_supported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraProp {
    pub name: String,
    #[serde(default)]
    pub is_required: bool,
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestHints {
    #[serde(default)]
    pub adult: bool,
    #[serde(default)]
    pub p2p: bool,
    #[serde(default)]
    pub configurable: bool,
    #[serde(default)]
    pub configuration_required: bool,
}

impl Catalog {
    pub fn supports(&self, extra: &str) -> bool {
        self.extra.iter().any(|e| e.name == extra) || self.extra_supported.iter().any(|e| e == extra)
    }

    pub fn has_required_extra(&self) -> bool {
        !self.extra_required.is_empty() || self.extra.iter().any(|e| e.is_required)
    }

    pub fn genres(&self) -> &[String] {
        self.extra.iter().find(|e| e.name == "genre").map(|e| e.options.as_slice()).unwrap_or_default()
    }

    fn genre_required(&self) -> bool {
        self.extra_required.iter().any(|e| e == "genre") || self.extra.iter().any(|e| e.name == "genre" && e.is_required)
    }

    /// Browsable without user input: any required extra is a genre we can pick a default for.
    pub fn browsable(&self) -> bool {
        let required = self.extra.iter().filter(|e| e.is_required).map(|e| e.name.as_str()).chain(self.extra_required.iter().map(String::as_str));
        required.into_iter().all(|name| name == "genre" && !self.genres().is_empty())
    }

    /// A page of this catalog, filtered by `genre`; a required genre defaults to the first option.
    pub fn discover_request(&self, genre: Option<&str>, skip: usize) -> ResourceRequest {
        let genre = genre.or_else(|| self.genre_required().then(|| self.genres().first().map(String::as_str)).flatten());
        let mut req = ResourceRequest::new("catalog", &self.r#type, &self.id);
        if let Some(g) = genre {
            req = req.extra("genre", g);
        }
        if skip > 0 {
            req = req.extra("skip", &skip.to_string());
        }
        req
    }
}

/// Catalogs of `kind` a user can browse, in addon order: `(addon index, catalog)`.
pub fn discover_catalogs<'a>(addons: &'a [Addon], kind: &'a str) -> impl Iterator<Item = (usize, &'a Catalog)> {
    addons
        .iter()
        .enumerate()
        .flat_map(move |(i, a)| a.manifest.catalogs.iter().filter(move |c| c.r#type == kind && c.browsable()).map(move |c| (i, c)))
}

impl Manifest {
    /// Catalogs that can show as a Home row: no input needed.
    pub fn home_catalogs(&self) -> impl Iterator<Item = &Catalog> {
        self.catalogs.iter().filter(|c| !c.has_required_extra())
    }

    pub fn search_catalogs(&self) -> impl Iterator<Item = &Catalog> {
        self.catalogs.iter().filter(|c| c.supports("search"))
    }
}

/// An installed addon: where it lives and what it declared.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct Addon {
    pub manifest_url: Url,
    pub manifest: Manifest,
}

/// `{base}/{resource}/{type}/{id}[/{extra}].json`
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceRequest {
    pub resource: String,
    pub r#type: String,
    pub id: String,
    pub extra: Vec<(String, String)>,
}

impl ResourceRequest {
    pub fn new(resource: &str, r#type: &str, id: &str) -> Self {
        Self { resource: resource.into(), r#type: r#type.into(), id: id.into(), extra: vec![] }
    }

    pub fn extra(mut self, name: &str, value: &str) -> Self {
        self.extra.push((name.into(), value.into()));
        self
    }

    pub fn url(&self, addon: &Addon) -> String {
        let mut path = format!(
            "{}/{}/{}/{}",
            base_url(&addon.manifest_url),
            self.resource,
            utf8_percent_encode(&self.r#type, ID),
            utf8_percent_encode(&self.id, ID),
        );
        for (i, (name, value)) in self.extra.iter().enumerate() {
            path.push(if i == 0 { '/' } else { '&' });
            path.push_str(&format!("{}={}", utf8_percent_encode(name, COMPONENT), utf8_percent_encode(value, COMPONENT)));
        }
        path + ".json"
    }
}

impl Addon {
    /// Whether this addon should be asked for `req`, following Stremio's rules:
    /// catalogs must be declared; other resources match on name, type and id prefix.
    pub fn serves(&self, req: &ResourceRequest) -> bool {
        let m = &self.manifest;
        if req.resource == "catalog" {
            return m.catalogs.iter().any(|c| c.r#type == req.r#type && c.id == req.id);
        }
        m.resources.iter().any(|r| {
            let (name, types, prefixes) = match r {
                Resource::Short(name) => (name, &m.types, m.id_prefixes.as_ref()),
                Resource::Full { name, types, id_prefixes } => (
                    name,
                    types.as_ref().unwrap_or(&m.types),
                    id_prefixes.as_ref().or(m.id_prefixes.as_ref()),
                ),
            };
            *name == req.resource
                && types.contains(&req.r#type)
                && prefixes.is_none_or(|p| p.iter().any(|p| req.id.starts_with(p.as_str())))
        })
    }
}

/// Every addon that serves `req`, in addon-list order, with the URL to fetch.
pub fn route<'a>(addons: &'a [Addon], req: &'a ResourceRequest) -> impl Iterator<Item = (&'a Addon, String)> {
    addons.iter().filter(|a| a.serves(req)).map(|a| (a, req.url(a)))
}

/// Cache hints any addon response may carry, in seconds.
#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheHints {
    pub cache_max_age: Option<u64>,
    pub stale_revalidate: Option<u64>,
    pub stale_error: Option<u64>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct CatalogResponse {
    pub metas: Vec<MetaPreview>,
    #[serde(flatten)]
    pub cache: CacheHints,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MetaResponse {
    pub meta: Meta,
    #[serde(flatten)]
    pub cache: CacheHints,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct StreamsResponse {
    pub streams: Vec<Stream>,
    #[serde(flatten)]
    pub cache: CacheHints,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SubtitlesResponse {
    pub subtitles: Vec<Subtitle>,
    #[serde(flatten)]
    pub cache: CacheHints,
}

/// A title as it appears in a catalog row.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPreview {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub description: Option<String>,
    pub release_info: Option<String>,
}

/// A title's full details.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub logo: Option<String>,
    pub description: Option<String>,
    pub release_info: Option<String>,
    pub runtime: Option<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub videos: Vec<Video>,
    /// Addons send these as a number or a string.
    pub imdb_rating: Option<serde_json::Value>,
    /// People fields come as a list or a single string; newer addons only use `links`.
    pub cast: Option<serde_json::Value>,
    pub director: Option<serde_json::Value>,
    pub writer: Option<serde_json::Value>,
    #[serde(default)]
    pub links: Vec<MetaLink>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct MetaLink {
    pub name: String,
    pub category: String,
}

impl Meta {
    pub fn rating(&self) -> Option<String> {
        match self.imdb_rating.as_ref()? {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        }
    }

    /// Names from `field` (`cast`, `director`, `writer`), else from `links` of `category`.
    pub fn people(&self, field: &str, category: &str) -> Vec<String> {
        let value = match field {
            "cast" => &self.cast,
            "director" => &self.director,
            _ => &self.writer,
        };
        let names: Vec<String> = match value {
            Some(serde_json::Value::String(s)) => s.split(',').map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).collect(),
            Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            _ => vec![],
        };
        if !names.is_empty() {
            return names;
        }
        self.links.iter().filter(|l| l.category == category).map(|l| l.name.clone()).collect()
    }
}

/// An episode (or other video) of a title.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Video {
    pub id: String,
    pub title: Option<String>,
    /// Cinemeta sends `name` instead of `title`.
    pub name: Option<String>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub released: Option<String>,
    pub thumbnail: Option<String>,
    pub overview: Option<String>,
}

impl Video {
    pub fn title(&self) -> &str {
        self.title.as_deref().or(self.name.as_deref()).unwrap_or_default()
    }
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    pub name: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(flatten)]
    pub source: StreamSource,
    #[serde(default)]
    pub behavior_hints: StreamHints,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(untagged, rename_all_fields = "camelCase")]
pub enum StreamSource {
    Url { url: String },
    YouTube { yt_id: String },
    Torrent {
        info_hash: String,
        file_idx: Option<u32>,
        #[serde(default)]
        sources: Vec<String>,
    },
    External { external_url: String },
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamHints {
    #[serde(default)]
    pub not_web_ready: bool,
    pub binge_group: Option<String>,
    pub filename: Option<String>,
    pub proxy_headers: Option<ProxyHeaders>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, serde::Serialize)]
pub struct ProxyHeaders {
    #[serde(default)]
    pub request: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub response: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct Subtitle {
    pub id: String,
    pub url: String,
    pub lang: String,
}

/// A series' episodes grouped by season: regular seasons ascending, specials (season 0) last.
pub fn seasons(videos: &[Video]) -> Vec<(u32, Vec<&Video>)> {
    let mut map = std::collections::BTreeMap::<u32, Vec<&Video>>::new();
    for v in videos {
        if let Some(season) = v.season {
            map.entry(season).or_default().push(v);
        }
    }
    for eps in map.values_mut() {
        eps.sort_by_key(|v| v.episode);
    }
    let mut out: Vec<_> = map.into_iter().collect();
    let specials_first = out.first().is_some_and(|(s, _)| *s == 0);
    out.rotate_left(usize::from(specials_first));
    out
}

/// Stream responses keyed by the answering addon's index, ordered by addon-list order.
pub fn stream_groups(mut arrived: Vec<(usize, Vec<Stream>)>) -> Vec<(usize, Vec<Stream>)> {
    arrived.sort_by_key(|(i, _)| *i);
    arrived
}

impl Stream {
    /// The URL mpv can play directly. Torrents, YouTube and external links aren't playable yet.
    pub fn playable_url(&self) -> Option<&str> {
        match &self.source {
            StreamSource::Url { url } => Some(url),
            _ => None,
        }
    }
}

/// One request per catalog that supports `search`, in addon order: `(addon index, catalog, request)`.
pub fn search_requests<'a>(addons: &'a [Addon], query: &str) -> Vec<(usize, &'a Catalog, ResourceRequest)> {
    let query = query.trim();
    if query.is_empty() {
        return vec![];
    }
    addons
        .iter()
        .enumerate()
        .flat_map(|(i, a)| a.manifest.search_catalogs().map(move |c| (i, c)))
        .map(|(i, c)| (i, c, ResourceRequest::new("catalog", &c.r#type, &c.id).extra("search", query)))
        .collect()
}

/// Subtitle requests for a video, one per addon that serves them: `(addon index, request)`.
/// `filename` is the stream's `behaviorHints.filename`, which helps addons match the release.
pub fn subtitle_requests(addons: &[Addon], kind: &str, video_id: &str, filename: Option<&str>) -> Vec<(usize, ResourceRequest)> {
    let mut req = ResourceRequest::new("subtitles", kind, video_id);
    if let Some(f) = filename {
        req = req.extra("filename", f);
    }
    addons.iter().enumerate().filter(|(_, a)| a.serves(&req)).map(|(i, _)| (i, req.clone())).collect()
}

/// The addons a signed-in user actually uses: their own, then the server admin's (when switched on),
/// skipping any admin addon the user installed themselves, since their copy carries their config.
pub fn with_admin_addons(user: &[Addon], admin: &[Addon], enabled: bool) -> Vec<Addon> {
    let extra = admin.iter().filter(|a| enabled && !user.iter().any(|u| u.manifest.id == a.manifest.id));
    user.iter().chain(extra).cloned().collect()
}

/// Relix doesn't play torrents (ADR 0008), so infoHash streams are dropped before display.
/// Debrid-configured addons return URL streams instead, which are kept.
pub fn without_torrents(streams: Vec<Stream>) -> Vec<Stream> {
    streams.into_iter().filter(|s| !matches!(s.source, StreamSource::Torrent { .. })).collect()
}

/// The stream to auto-play for the next episode: a playable stream from the same release
/// (`bingeGroup`) as the one just watched, from the same addon if it has one, else the first
/// addon in order. `None` means don't auto-play.
pub fn pick_binge_stream<'a>(groups: &'a [(usize, Vec<Stream>)], prev_addon: usize, binge_group: &str) -> Option<(usize, &'a Stream)> {
    let matches = |s: &&Stream| s.playable_url().is_some() && s.behavior_hints.binge_group.as_deref() == Some(binge_group);
    let in_addon = |(a, streams): &'a (usize, Vec<Stream>)| streams.iter().find(matches).map(|s| (*a, s));
    groups.iter().filter(|(a, _)| *a == prev_addon).find_map(in_addon).or_else(|| groups.iter().find_map(in_addon))
}
