//! TMDB (themoviedb.org) as the main Movies/Series catalog: its discover search filters the whole
//! catalog by genre, year, rating and sort on TMDB's side, unlike addon catalogs, which are
//! filtered only as far as they're loaded. Titles carry their IMDb ids, so addons find streams.
//!
//! The read token comes in when building (`RELIX_TMDB_TOKEN`, a repository secret); without it
//! Browse shows the addon catalogs only. "This product uses the TMDB API but is not endorsed or
//! certified by TMDB" (Settings says so, per TMDB's terms).

use relix_proto::addon::MetaPreview;
use serde::Deserialize;
use tokio::task::JoinSet;

use crate::filter::{Filter, Sort};

const TOKEN: Option<&str> = option_env!("RELIX_TMDB_TOKEN");
// TMDB's other API address: api.themoviedb.org resets about half of new connections from some
// networks (seen from India: 9 of 20 failed, api.tmdb.org 0 of 20).
const API: &str = "https://api.tmdb.org/3";

pub fn enabled() -> bool {
    TOKEN.is_some_and(|t| !t.is_empty())
}

/// TMDB's genres for "movie" or "series" (TMDB calls series "tv"), as (id, name).
pub fn genres(kind: &str) -> &'static [(u32, &'static str)] {
    const MOVIE: &[(u32, &str)] = &[
        (28, "Action"), (12, "Adventure"), (16, "Animation"), (35, "Comedy"), (80, "Crime"), (99, "Documentary"),
        (18, "Drama"), (10751, "Family"), (14, "Fantasy"), (36, "History"), (27, "Horror"), (10402, "Music"),
        (9648, "Mystery"), (10749, "Romance"), (878, "Science Fiction"), (10770, "TV Movie"), (53, "Thriller"),
        (10752, "War"), (37, "Western"),
    ];
    const TV: &[(u32, &str)] = &[
        (10759, "Action & Adventure"), (16, "Animation"), (35, "Comedy"), (80, "Crime"), (99, "Documentary"),
        (18, "Drama"), (10751, "Family"), (10762, "Kids"), (9648, "Mystery"), (10763, "News"), (10764, "Reality"),
        (10765, "Sci-Fi & Fantasy"), (10766, "Soap"), (10767, "Talk"), (10768, "War & Politics"), (37, "Western"),
    ];
    if kind == "series" { TV } else { MOVIE }
}

/// The Country filter's choices: (ISO 3166-1 code, name).
// shortcut: the main film and TV countries only, not TMDB's ~250; add any that viewers ask for.
pub const COUNTRIES: &[(&str, &str)] = &[
    ("US", "United States"), ("GB", "United Kingdom"), ("IN", "India"), ("KR", "South Korea"), ("JP", "Japan"),
    ("CN", "China"), ("HK", "Hong Kong"), ("TW", "Taiwan"), ("TH", "Thailand"), ("PH", "Philippines"),
    ("ID", "Indonesia"), ("PK", "Pakistan"), ("TR", "Turkey"), ("IR", "Iran"), ("EG", "Egypt"),
    ("NG", "Nigeria"), ("ZA", "South Africa"), ("FR", "France"), ("DE", "Germany"), ("ES", "Spain"),
    ("IT", "Italy"), ("NL", "Netherlands"), ("BE", "Belgium"), ("SE", "Sweden"), ("DK", "Denmark"),
    ("NO", "Norway"), ("FI", "Finland"), ("PL", "Poland"), ("RU", "Russia"), ("UA", "Ukraine"),
    ("IE", "Ireland"), ("CA", "Canada"), ("MX", "Mexico"), ("BR", "Brazil"), ("AR", "Argentina"),
    ("CO", "Colombia"), ("CL", "Chile"), ("AU", "Australia"), ("NZ", "New Zealand"),
];

/// The discover query for one page: the filter's genres, years, rating and sort.
fn discover_url(kind: &str, f: &Filter, page: u32) -> String {
    let tv = kind == "series";
    let ids: Vec<String> = f.genres.iter().filter_map(|g| genres(kind).iter().find(|(_, n)| n == g)).map(|(id, _)| id.to_string()).collect();
    // A comma is "all of these", a pipe "any of these".
    let mut q = vec![format!("page={page}"), "include_adult=false".into()];
    if !ids.is_empty() {
        q.push(format!("with_genres={}", ids.join(if f.match_any { "|" } else { "," })));
    }
    let date = if tv { "first_air_date" } else { "primary_release_date" };
    if let Some((from, to)) = f.years {
        q.push(format!("{date}.gte={from}-01-01"));
        q.push(format!("{date}.lte={to}-12-31"));
    }
    // Without a vote floor, obscure titles with a handful of votes top every list (one 10 is
    // "top rated"; announced films are "newest"). The floor depends on the sort.
    let floor = match f.sort {
        Sort::Rating => 500,
        Sort::Name => 200,
        Sort::Newest => 100,
        Sort::Catalog => if f.min_rating.is_some() { 100 } else { 0 },
    };
    if let Some(min) = f.min_rating {
        q.push(format!("vote_average.gte={min}"));
    }
    if floor > 0 {
        q.push(format!("vote_count.gte={floor}"));
    }
    let sort = match f.sort {
        Sort::Catalog => "popularity.desc",
        Sort::Newest => if tv { "first_air_date.desc" } else { "primary_release_date.desc" },
        Sort::Rating => "vote_average.desc",
        Sort::Name => if tv { "name.asc" } else { "title.asc" },
    };
    q.push(format!("sort_by={sort}"));
    // Animation is genre 16 for movies and series alike; anime is filed under it.
    if f.hide_animation {
        q.push("without_genres=16".into());
    }
    if let Some(code) = f.country {
        q.push(format!("with_origin_country={code}"));
    }
    format!("{API}/discover/{}?{}", if tv { "tv" } else { "movie" }, q.join("&"))
}

#[derive(Deserialize)]
struct Page {
    page: u32,
    total_pages: u32,
    results: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    id: u64,
    #[serde(alias = "name")]
    title: String,
    overview: Option<String>,
    poster_path: Option<String>,
    backdrop_path: Option<String>,
    #[serde(alias = "first_air_date")]
    release_date: Option<String>,
    #[serde(default)]
    genre_ids: Vec<u32>,
    vote_average: Option<f64>,
}

#[derive(Deserialize)]
struct ExternalIds {
    imdb_id: Option<String>,
}

/// TMDB allows about 40 requests a second; Home asks for ~150 at once (a page is ~21).
static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);

async fn get<T: serde::de::DeserializeOwned>(http: &reqwest::Client, url: String) -> Result<T, String> {
    let _slot = SLOTS.acquire().await.map_err(|e| e.to_string())?;
    let res = http.get(url).bearer_auth(TOKEN.unwrap_or_default()).timeout(std::time::Duration::from_secs(20)).send().await.map_err(|e| e.without_url().to_string())?;
    res.error_for_status().map_err(|e| format!("TMDB: {}", e.without_url()))?.json().await.map_err(|e| e.without_url().to_string())
}

/// One page of the filtered catalog, and whether there are more. Titles without an IMDb id are
/// left out: addons can't find streams for them.
pub async fn discover(http: &reqwest::Client, kind: &str, f: &Filter, page: u32) -> Result<(Vec<MetaPreview>, bool), String> {
    let found: Page = get(http, discover_url(kind, f, page)).await?;
    // TMDB stops at page 500.
    let more = found.page < found.total_pages.min(500);
    Ok((with_imdb_ids(http, kind, found.results).await, more))
}

/// What's trending on TMDB this week: a Home row.
pub async fn trending(http: &reqwest::Client, kind: &str) -> Result<Vec<MetaPreview>, String> {
    let found: Page = get(http, format!("{API}/trending/{}/week", if kind == "series" { "tv" } else { "movie" })).await?;
    Ok(with_imdb_ids(http, kind, found.results).await)
}

/// IMDb ids by (TMDB type, TMDB id); `None` for titles without one.
type ImdbIds = std::collections::BTreeMap<(&'static str, u64), Option<String>>;
static IMDB: std::sync::Mutex<ImdbIds> = std::sync::Mutex::new(ImdbIds::new());

/// A title the viewer watched, saved or turned down: what recommendations start from. `weight`
/// is how much it says about their taste (a finished title counts double; "Not interested" is
/// negative, pushing titles like it down).
#[derive(Clone)]
pub struct Seed {
    pub imdb: String,
    pub kind: String,
    pub weight: i32,
}

#[derive(Deserialize)]
struct Found {
    id: u64,
}

#[derive(Deserialize)]
struct FindResults {
    #[serde(default)]
    movie_results: Vec<Found>,
    #[serde(default)]
    tv_results: Vec<Found>,
}

/// TMDB's entry for an IMDb id.
async fn find(http: &reqwest::Client, imdb: &str, kind: &str) -> Option<Found> {
    let r: FindResults = get(http, format!("{API}/find/{imdb}?external_source=imdb_id")).await.ok()?;
    if kind == "series" { r.tv_results } else { r.movie_results }.into_iter().next()
}

/// TMDB's lists for one seed: what its viewers also liked (weighted double) and what's similar.
async fn lists_for(http: &reqwest::Client, seed: &Seed) -> Vec<(i32, Vec<Item>)> {
    let Some(found) = find(http, &seed.imdb, &seed.kind).await else { return vec![] };
    let tmdb_kind = if seed.kind == "series" { "tv" } else { "movie" };
    let (recs, similar) = tokio::join!(
        get::<Page>(http, format!("{API}/{tmdb_kind}/{}/recommendations", found.id)),
        get::<Page>(http, format!("{API}/{tmdb_kind}/{}/similar", found.id)),
    );
    [(2, recs), (1, similar)].into_iter().filter_map(|(w, page)| page.ok().map(|p| (w * seed.weight, p.results))).collect()
}

/// "More like this" and "Because you watched X": titles like one title, ranked.
pub async fn more_like(http: &reqwest::Client, seed: &Seed) -> Result<Vec<MetaPreview>, String> {
    let lists = lists_for(http, seed).await;
    if lists.is_empty() {
        return Err("not on TMDB".into());
    }
    Ok(with_imdb_ids(http, &seed.kind, rank(lists)).await)
}

/// "For You": what viewers of several of the seeds (one type) also liked, ranked by how many
/// seeds point to it, how high, its rating, and how much each seed says (item-based
/// collaborative filtering: TMDB's recommendations do the learning). Turned-down seeds subtract.
pub async fn for_you(http: &reqwest::Client, kind: &str, seeds: &[Seed]) -> Result<Vec<MetaPreview>, String> {
    let mut set = JoinSet::new();
    for seed in seeds {
        let (http, seed) = (http.clone(), seed.clone());
        set.spawn(async move { lists_for(&http, &seed).await });
    }
    let mut lists = vec![];
    while let Some(Ok(more)) = set.join_next().await {
        lists.extend(more);
    }
    Ok(with_imdb_ids(http, kind, rank(lists)).await)
}

/// The 20 titles the lists agree on most: each list adds its weight, more for a higher place.
fn rank(lists: Vec<(i32, Vec<Item>)>) -> Vec<Item> {
    let mut scores: std::collections::HashMap<u64, (f32, Item)> = Default::default();
    for (weight, items) in lists {
        for (place, item) in items.into_iter().enumerate() {
            // TMDB's lists include weak films (Inception's starts with ones rated under 6): a
            // well-rated title counts several times a mediocre one.
            let quality = (item.vote_average.unwrap_or(5.) as f32 - 5.).clamp(0.3, 4.);
            let points = weight as f32 * (1. - place as f32 / 25.) * quality;
            scores.entry(item.id).or_insert((0., item)).0 += points;
        }
    }
    let mut ranked: Vec<(f32, Item)> = scores.into_values().collect();
    ranked.retain(|(score, _)| *score > 0.);
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.id.cmp(&b.1.id)));
    ranked.into_iter().take(20).map(|(_, item)| item).collect()
}

async fn with_imdb_ids(http: &reqwest::Client, kind: &str, results: Vec<Item>) -> Vec<MetaPreview> {
    let tmdb_kind = if kind == "series" { "tv" } else { "movie" };
    // TMDB's ids aren't IMDb's: one lookup per title, all at once.
    let mut set = JoinSet::new();
    let mut imdb = vec![None; results.len()];
    for (n, item) in results.iter().enumerate() {
        // Ids never change: each is looked up once per run.
        if let Some(known) = IMDB.lock().unwrap().get(&(tmdb_kind, item.id)) {
            imdb[n] = known.clone();
            continue;
        }
        let (http, url, id) = (http.clone(), format!("{API}/{tmdb_kind}/{}/external_ids", item.id), item.id);
        set.spawn(async move { (n, id, get::<ExternalIds>(&http, url).await.map(|e| e.imdb_id.filter(|id| id.starts_with("tt")))) });
    }
    while let Some(Ok((n, id, found))) = set.join_next().await {
        if let Ok(found) = found {
            IMDB.lock().unwrap().insert((tmdb_kind, id), found.clone());
            imdb[n] = found;
        }
    }
    let image = |size: &str, path: &Option<String>| path.as_ref().map(|p| format!("https://image.tmdb.org/t/p/{size}{p}"));
    results
        .into_iter()
        .zip(imdb)
        .filter_map(|(item, id)| {
            Some(MetaPreview {
                id: id?,
                r#type: kind.to_owned(),
                poster: image("w342", &item.poster_path),
                background: image("w1280", &item.backdrop_path),
                logo: None,
                description: item.overview.filter(|o| !o.is_empty()),
                release_info: item.release_date.and_then(|d| d.get(..4).map(str::to_owned)),
                genres: item.genre_ids.iter().filter_map(|g| genres(kind).iter().find(|(id, _)| id == g)).map(|(_, n)| (*n).to_owned()).collect(),
                imdb_rating: item.vote_average.filter(|v| *v > 0.).map(|v| serde_json::json!(format!("{v:.1}"))),
                name: item.title,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_several_seeds_recommend_rank_first() {
        let item = |id: u64| Item { id, title: id.to_string(), overview: None, poster_path: None, backdrop_path: None, release_date: None, genre_ids: vec![], vote_average: Some(7.) };
        // 7 is second on two lists; 1 tops just one.
        let lists = vec![(1, vec![item(1), item(7)]), (1, vec![item(2), item(7)])];
        assert_eq!(rank(lists).iter().map(|i| i.id).collect::<Vec<_>>(), [7, 1, 2]);
        // A finished seed (weight 2) outweighs a half-watched one.
        let lists = vec![(2, vec![item(3)]), (1, vec![item(4)])];
        assert_eq!(rank(lists)[0].id, 3);
        // A top-rated title low on a list beats a poorly rated one at the top.
        let rated = |id, v| Item { vote_average: Some(v), ..item(id) };
        assert_eq!(rank(vec![(1, vec![rated(5, 5.5), rated(6, 8.4)])])[0].id, 6);
        // "Not interested" pushes what it points to out.
        assert_eq!(rank(vec![(1, vec![item(8), item(9)]), (-2, vec![item(8)])]).iter().map(|i| i.id).collect::<Vec<_>>(), [9]);
    }

    #[test]
    fn filters_become_one_discover_query() {
        let f = Filter { genres: vec!["Comedy".into(), "Drama".into()], years: Some((2009, 2010)), min_rating: Some(7.), ..Filter::default() };
        let url = discover_url("movie", &f, 2);
        assert!(url.starts_with("https://api.tmdb.org/3/discover/movie?page=2&"), "{url}");
        assert!(discover_url("movie", &Filter { country: Some("IN"), ..Filter::default() }, 1).contains("with_origin_country=IN"));
        assert!(discover_url("series", &Filter { hide_animation: true, ..Filter::default() }, 1).contains("without_genres=16"));
        for part in ["with_genres=35,18", "primary_release_date.gte=2009-01-01", "primary_release_date.lte=2010-12-31", "vote_average.gte=7", "vote_count.gte=100", "sort_by=popularity.desc"] {
            assert!(url.contains(part), "{part} in {url}");
        }
        let any = discover_url("series", &Filter { genres: vec!["Comedy".into(), "Crime".into()], match_any: true, sort: Sort::Name, ..Filter::default() }, 1);
        assert!(any.contains("/discover/tv?") && any.contains("with_genres=35|80") && any.contains("sort_by=name.asc"), "{any}");
    }
}
