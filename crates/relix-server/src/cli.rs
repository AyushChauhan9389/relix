//! `relix-server admin ...`: manage the addons every signed-in account receives.

use relix_proto::addon::manifest_url;
use url::Url;

pub const USAGE: &str = "usage: relix-server admin <command>
  add <manifest-url>   publish an addon to every signed-in account
  list                 show published addons and their ids
  remove <id>          stop publishing an addon
env: RELIX_ADMIN_TOKEN (required), RELIX_SERVER_URL (default http://127.0.0.1:8787)";

#[derive(Debug, PartialEq)]
pub enum AdminCommand {
    Add(Url),
    List,
    Remove(u64),
}

/// Parses the arguments after `admin`.
pub fn parse_admin(args: &[String]) -> Result<AdminCommand, String> {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["add", url] => manifest_url(url)
            .map(AdminCommand::Add)
            .map_err(|_| format!("{url} isn't an addon link; it should end in /manifest.json")),
        ["list"] => Ok(AdminCommand::List),
        ["remove", id] => id.parse().map(AdminCommand::Remove).map_err(|_| format!("{id} isn't an addon id; see `list`")),
        _ => Err(USAGE.into()),
    }
}
