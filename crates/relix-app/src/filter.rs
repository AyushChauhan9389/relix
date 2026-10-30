//! Filters for Movies/Series, applied to the titles already loaded: addons accept one genre per
//! request, so combining genres, years, ratings and sorting happens here.

use std::collections::HashSet;

use relix_proto::addon::MetaPreview;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    pub genres: Vec<String>,
    /// Any of `genres` instead of all of them.
    pub match_any: bool,
    /// Inclusive.
    pub years: Option<(u32, u32)>,
    pub min_rating: Option<f32>,
    pub sort: Sort,
    pub hide_watched: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Sort {
    #[default]
    Catalog,
    Newest,
    Rating,
    Name,
}

impl Sort {
    pub const ALL: [(Sort, &'static str); 4] =
        [(Sort::Catalog, "Catalog order"), (Sort::Newest, "Newest"), (Sort::Rating, "Top rated"), (Sort::Name, "A–Z")];
}

impl Filter {
    /// Something is filtered or sorted (the All/Any switch alone changes nothing).
    pub fn is_active(&self) -> bool {
        Filter { match_any: false, ..self.clone() } != Filter::default()
    }
}

/// The titles that pass `f`, in its sort order. `watched` holds finished titles' ids.
pub fn apply(items: &[MetaPreview], f: &Filter, watched: &HashSet<String>) -> Vec<MetaPreview> {
    let has = |m: &MetaPreview, g: &String| m.genres.iter().any(|mg| mg.eq_ignore_ascii_case(g));
    let mut out: Vec<MetaPreview> = items
        .iter()
        .filter(|m| {
            f.genres.is_empty()
                || if f.match_any { f.genres.iter().any(|g| has(m, g)) } else { f.genres.iter().all(|g| has(m, g)) }
        })
        .filter(|m| f.years.is_none_or(|(from, to)| m.year().is_some_and(|y| (from..=to).contains(&y))))
        .filter(|m| f.min_rating.is_none_or(|min| m.rating().is_some_and(|r| r >= min)))
        .filter(|m| !(f.hide_watched && watched.contains(&m.id)))
        .cloned()
        .collect();
    // Stable sorts; titles missing the sort key go last.
    match f.sort {
        Sort::Catalog => {}
        Sort::Newest => out.sort_by_key(|m| std::cmp::Reverse(m.year().unwrap_or(0))),
        Sort::Rating => out.sort_by(|a, b| b.rating().unwrap_or(-1.).total_cmp(&a.rating().unwrap_or(-1.))),
        Sort::Name => out.sort_by_key(|m| m.name.to_lowercase()),
    }
    out
}

/// Earliest and latest year among the titles, for the year range's choices.
pub fn year_bounds(items: &[MetaPreview]) -> Option<(u32, u32)> {
    let years = items.iter().filter_map(MetaPreview::year);
    Some((years.clone().min()?, years.max()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn title(id: &str, genres: &[&str], year: &str, rating: Option<&str>) -> MetaPreview {
        serde_json::from_value(serde_json::json!({
            "id": id, "type": "movie", "name": id,
            "genres": genres, "releaseInfo": year, "imdbRating": rating,
        }))
        .unwrap()
    }

    fn items() -> Vec<MetaPreview> {
        vec![
            title("b-action", &["Action"], "2010", Some("6.5")),
            title("a-action-comedy", &["Action", "Comedy"], "2020", Some("8.1")),
            title("c-comedy", &["Comedy"], "1999", None),
            title("d-drama", &["Drama"], "2015–2018", Some("7.2")),
        ]
    }

    fn ids(f: &Filter, watched: &[&str]) -> Vec<String> {
        let watched = watched.iter().map(|s| s.to_string()).collect();
        apply(&items(), f, &watched).into_iter().map(|m| m.id).collect()
    }

    fn genres(g: &[&str]) -> Vec<String> {
        g.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_filters_keeps_catalog_order() {
        assert_eq!(ids(&Filter::default(), &[]), ["b-action", "a-action-comedy", "c-comedy", "d-drama"]);
    }

    #[test]
    fn genres_match_all_or_any() {
        let all = Filter { genres: genres(&["Action", "Comedy"]), ..Default::default() };
        assert_eq!(ids(&all, &[]), ["a-action-comedy"]);
        let any = Filter { match_any: true, ..all };
        assert_eq!(ids(&any, &[]), ["b-action", "a-action-comedy", "c-comedy"]);
    }

    #[test]
    fn year_range_and_minimum_rating_drop_titles_without_them() {
        let years = Filter { years: Some((2010, 2016)), ..Default::default() };
        assert_eq!(ids(&years, &[]), ["b-action", "d-drama"]);
        let rated = Filter { min_rating: Some(7.), ..Default::default() };
        assert_eq!(ids(&rated, &[]), ["a-action-comedy", "d-drama"]);
    }

    #[test]
    fn hide_watched() {
        let f = Filter { hide_watched: true, ..Default::default() };
        assert_eq!(ids(&f, &["c-comedy", "b-action"]), ["a-action-comedy", "d-drama"]);
    }

    #[test]
    fn sorts() {
        let by = |sort| ids(&Filter { sort, ..Default::default() }, &[]);
        assert_eq!(by(Sort::Newest), ["a-action-comedy", "d-drama", "b-action", "c-comedy"]);
        assert_eq!(by(Sort::Rating), ["a-action-comedy", "d-drama", "b-action", "c-comedy"]);
        assert_eq!(by(Sort::Name), ["a-action-comedy", "b-action", "c-comedy", "d-drama"]);
    }

    #[test]
    fn year_bounds_come_from_the_titles() {
        assert_eq!(year_bounds(&items()), Some((1999, 2020)));
        assert_eq!(year_bounds(&[]), None);
    }
}
