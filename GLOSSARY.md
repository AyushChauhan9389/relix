# Relix

A desktop streaming app that browses and plays content from Stremio-compatible addons, with profiles and private, synced user data.

## Addons

**Addon**:
A remote service, identified by its manifest URL, that supplies catalogs, metadata, streams or subtitles.
_Avoid_: plugin, extension, provider

**Manifest**:
An addon's self-description: its id, name, the resources and types it serves, and its catalogs.

**Addon list**:
The ordered set of addons a profile has installed. Order decides the order of Home rows and stream groups.
_Avoid_: addon collection

**Catalog**:
A named, pageable list of titles an addon offers for one type, e.g. "Popular Movies".

**Title**:
A movie, series or other item that a catalog lists and meta describes. Identified by an id such as `tt0903747`.
_Avoid_: item, content, media

**Episode**:
One video of a series title, identified as `{titleId}:{season}:{episode}`.

**Stream**:
One playable source for a title or episode that an addon returns: a URL, a YouTube id, a torrent infoHash or an external link.
_Avoid_: link, source, file

## People and data

**Account**:
A sign-in identity on a sync server. Owns profiles. Optional: Relix works fully without one.
_Avoid_: user (ambiguous with profile)

**Profile**:
One person's space inside Relix. It has its own addon list, library, progress, watched flags and settings.
_Avoid_: user

**Library**:
The titles a profile has saved, shown as "My List".
_Avoid_: watchlist, favourites

**Progress**:
How far a profile got through a title or episode. It drives Continue Watching.

**Watched flag**:
The mark that a profile has finished an episode or movie.

**Record**:
The unit of sync. One piece of profile data, identified by profile, kind and key.


