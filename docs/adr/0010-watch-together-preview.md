# Watch Together: a closed preview that plays a host-provided link

Decided 2026-11-05. Parties play one video for everyone from a link the host provides, mainly a Google Drive share link (TorBox can upload finished downloads to Google Drive). Party state, sync and chat run over one WebSocket per app to the Relix server and live in its memory. Only accounts the admin enables can host.

Rejected: sharing the host's resolved debrid link, and relaying it through the host. Debrid providers ban accounts whose files stream from several IPs; relaying hides that, which is evasion; and a 4K stream needs 40-140 Mbps of the host's upload per viewer.

Rejected: each viewer playing from their own addons. It is safe, but viewers without a working addon can't watch, and releases differ.

## Consequences

- The risk moves to the host's Google account. Publicly shared copyrighted files can be removed or get the account suspended, and heavily streamed files hit Drive's download quota. Hosts are told to use a separate Google account and to unshare after the party.
- Deleting the Drive file after a party needs Google OAuth with the full Drive scope, because TorBox, not Relix, creates the files. An unverified Google project is limited to 100 listed test users.
- A server restart ends running parties. Friends and usernames are stored in SQLite as usual.
