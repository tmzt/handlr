#!/bin/sh
# Copies one agy (Antigravity CLI) conversation, read-only, into the
# gitignored out/sessions/agy_example/ (plan 20261023_06, step 1, as
# amended: the example is never committed). SQLite's own backup reads
# through the WAL, so the copy has the newest steps and is one file.
#
# Only the conversation's database is read; never agy's memory (brain/,
# annotations/, knowledge) or its OAuth token.
#
#   scripts/record-agy-example.sh <session id>
set -eu
id=${1:?usage: record-agy-example.sh <session id>}
case $id in
    *[!0-9a-f-]*) echo "not a session id: $id" >&2; exit 2 ;;
esac
src="$HOME/.gemini/antigravity-cli/conversations/$id.db"
[ -f "$src" ] || { echo "no conversation $id" >&2; exit 1; }
out="$(dirname "$0")/../out/sessions/agy_example"
mkdir -p "$out"
sqlite3 -readonly "$src" ".backup '$out/$id.db'"
echo "$out/$id.db"
