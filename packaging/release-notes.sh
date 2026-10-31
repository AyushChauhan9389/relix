#!/usr/bin/env bash
# Writes the release page for $1 (the version) to stdout: Gemini summarises the commits since the
# last release when GEMINI_API_KEY is set; otherwise (or if that fails) it's the plain commit list.
# The download table (packaging/release-notes.md) always follows.
set -uo pipefail
version=$1
prev=$(git describe --tags --abbrev=0 2>/dev/null || true)
commits=$(git log ${prev:+"$prev"..HEAD} --no-merges --format='- %s%n%b' | grep -v '^Co-Authored-By\|^Claude-Session\|^$' | head -c 30000)

notes=""
if [ -n "${GEMINI_API_KEY:-}" ]; then
  prompt="Write the GitHub release notes for Relix $version, a desktop streaming app compatible with Stremio addons.
Use these commits (since ${prev:-the first commit}). Output Markdown only: a one-line summary, then short sections
'New', 'Improved' and 'Fixed' with plain-English bullets a user understands (skip internal refactors, CI and docs).
Leave out any section with nothing in it. Don't invent anything that isn't in the commits, and keep every condition
the commits state: a feature that is off, needs setup, or only works somewhere is either left out or says so.

$commits"
  body=$(jq -n --arg p "$prompt" '{contents: [{parts: [{text: $p}]}]}')
  notes=$(curl -fsS --max-time 90 "https://generativelanguage.googleapis.com/v1beta/models/${GEMINI_MODEL:-gemini-flash-latest}:generateContent" \
    -H "x-goog-api-key: $GEMINI_API_KEY" -H 'content-type: application/json' -d "$body" \
    | jq -r '.candidates[0].content.parts[0].text // empty') || notes=""
  [ -n "$notes" ] || echo "Gemini didn't answer; using the commit list" >&2
fi
if [ -z "$notes" ]; then
  notes="## Changes since ${prev:-the first release}

$(git log ${prev:+"$prev"..HEAD} --no-merges --format='- %s')"
fi

printf '%s\n\n' "$notes"
cat packaging/release-notes.md
