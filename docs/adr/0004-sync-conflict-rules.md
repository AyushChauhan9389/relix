# Sync conflict rules

Each record (profile, kind, key) is last-write-wins, ordered by a server-assigned increasing sequence number; device clocks are never trusted. Clients push dirty records, pull changes since their last seq on connect, and receive live pushes. Two exceptions: watched flags only ever grow (merged by union, with "unwatched" stored as its own explicit entry) because losing a watched mark is the conflict users notice, and the addon list is a single record so concurrent reorders can't interleave. CRDTs were rejected: heavier storage and harder debugging.
