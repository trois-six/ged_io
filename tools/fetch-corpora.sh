#!/usr/bin/env bash
# Fetch the opt-in conformance corpora and the specification inputs listed in
# tests/fixtures/corpora.lock.tsv into target/corpora/<corpus>/<path>.
#
# None of these files is vendored: some carry licences that do not allow it
# (GPL, non-commercial, CC-BY-SA, none stated) and the rest are only needed to
# regenerate tables. Every file is pinned to a commit and checked against its
# SHA-256; a mismatch aborts.
#
# Usage:
#   tools/fetch-corpora.sh [CORPUS...]     # default: every corpus in the lock
#   tools/fetch-corpora.sh --registries    # also GEDCOM-registries (cross-check only)
#
# Then: cargo test --all-features --test conformance -- --ignored
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
lock="$root/tests/fixtures/corpora.lock.tsv"
dest="${CORPORA_DIR:-$root/target/corpora}"
registries_commit="05519e8aa4162843e737dd761df3a2aae7ea0860"

want_registries=0
corpora=()
for arg in "$@"; do
  case "$arg" in
    --registries) want_registries=1 ;;
    -h | --help) sed -n '2,15p' "$0"; exit 0 ;;
    *) corpora+=("$arg") ;;
  esac
done

if command -v sha256sum >/dev/null; then
  sha() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

selected() {
  [ "${#corpora[@]}" -eq 0 ] && return 0
  local c
  for c in "${corpora[@]}"; do [ "$c" = "$1" ] && return 0; done
  return 1
}

fetched=0
kept=0
while IFS=$'\t' read -r corpus path url sum _licence; do
  [ "$corpus" = "corpus" ] && continue
  selected "$corpus" || continue
  out="$dest/$corpus/$path"
  if [ -f "$out" ] && [ "$(sha "$out")" = "$sum" ]; then
    kept=$((kept + 1))
    continue
  fi
  mkdir -p "$(dirname "$out")"
  curl -fsSL --retry 3 -o "$out.part" "$url"
  got="$(sha "$out.part")"
  if [ "$got" != "$sum" ]; then
    rm -f "$out.part"
    echo "SHA-256 mismatch for $corpus/$path: expected $sum, got $got" >&2
    exit 1
  fi
  mv "$out.part" "$out"
  fetched=$((fetched + 1))
done <"$lock"
echo "corpora: $fetched fetched, $kept already present, in $dest"

if [ "$want_registries" -eq 1 ]; then
  reg="$dest/registries"
  if [ "$(git -C "$reg" rev-parse HEAD 2>/dev/null || true)" != "$registries_commit" ]; then
    rm -rf "$reg"
    git init -q "$reg"
    git -C "$reg" fetch -q --depth 1 https://github.com/FamilySearch/GEDCOM-registries "$registries_commit"
    git -C "$reg" checkout -q FETCH_HEAD
  fi
  echo "registries: $registries_commit in $reg"
fi
