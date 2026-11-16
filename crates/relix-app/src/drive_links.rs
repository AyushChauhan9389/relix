//! Google Drive links (shared by drive.rs and the web build, which has no Drive sign-in).

/// The Drive file id in a link: a share link (`/file/d/<id>/view`, `open?id=`) or a download one.
pub fn file_id(url: &str) -> Option<String> {
    let id = if let Some(rest) = url.split("/file/d/").nth(1) {
        rest.split(['/', '?', '#']).next()?
    } else {
        let is_drive = url.contains("drive.google.com") || url.contains("drive.usercontent.google.com") || url.contains("docs.google.com");
        let query = url.split_once('?')?.1;
        query.split('&').find_map(|kv| kv.strip_prefix("id=")).filter(|_| is_drive)?
    };
    let valid = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| id.to_owned())
}

/// The link a party keeps for a Drive file.
pub fn party_url(file_id: &str) -> String {
    format!("https://drive.google.com/file/d/{file_id}/view")
}
