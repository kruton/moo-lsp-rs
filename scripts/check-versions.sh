#!/usr/bin/env bash
set -euo pipefail

# Ensure we are in the repository root.
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

EXPECTED_TAG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag|--version)
      if [[ $# -lt 2 ]]; then
        echo "$1 requires a value" >&2
        exit 1
      fi
      EXPECTED_TAG="$2"
      shift 2
      ;;
    --*)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
    *)
      EXPECTED_TAG="$1"
      shift
      ;;
  esac
done

# Infer the expected version from CI or the environment when one was not passed.
if [[ -z "$EXPECTED_TAG" ]]; then
  if [[ "${GITHUB_REF_TYPE:-}" == "tag" && -n "${GITHUB_REF_NAME:-}" ]]; then
    EXPECTED_TAG="$GITHUB_REF_NAME"
  elif [[ "${GITHUB_REF:-}" == refs/tags/* ]]; then
    EXPECTED_TAG="${GITHUB_REF#refs/tags/}"
  elif [[ -n "${TAG:-}" ]]; then
    EXPECTED_TAG="$TAG"
  fi
fi

CLEAN_TAG="${EXPECTED_TAG#v}"

CARGO_VER=$(sed -n -E 's/^version = "(.*)"/\1/p' Cargo.toml 2>/dev/null | head -n1 || echo "")
CARGO_LOCK_VER=$(awk '/^name = "moo-lsp-rs"/ { found=1 } found && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.lock 2>/dev/null || echo "")
PKG_VER=$(jq -r '.version // empty' npm/package.json 2>/dev/null || echo "")
PKG_LOCK_VER1=$(jq -r '.version // empty' npm/package-lock.json 2>/dev/null || echo "")
PKG_LOCK_VER2=$(jq -r '.packages[""].version // empty' npm/package-lock.json 2>/dev/null || echo "")

MISMATCH=false
if [[ -z "$CARGO_VER" || "$CARGO_LOCK_VER" != "$CARGO_VER" || "$PKG_VER" != "$CARGO_VER" || "$PKG_LOCK_VER1" != "$CARGO_VER" || "$PKG_LOCK_VER2" != "$CARGO_VER" ]]; then
  MISMATCH=true
fi
if [[ -n "$CLEAN_TAG" && "$CARGO_VER" != "$CLEAN_TAG" ]]; then
  MISMATCH=true
fi

if [[ "$MISMATCH" == "true" ]]; then
  echo "Version mismatch detected!" >&2
  if [[ -n "$CLEAN_TAG" ]]; then
    echo "  Git tag:                       ${EXPECTED_TAG} (expected version: ${CLEAN_TAG})" >&2
  fi
  echo "  Cargo.toml:                    ${CARGO_VER:-<missing>}" >&2
  echo "  Cargo.lock:                    ${CARGO_LOCK_VER:-<missing>}" >&2
  echo "  npm/package.json:              ${PKG_VER:-<missing>}" >&2
  echo "  npm/package-lock.json:         ${PKG_LOCK_VER1:-<missing>}" >&2
  echo "  npm/package-lock.json package: ${PKG_LOCK_VER2:-<missing>}" >&2
  exit 1
fi

if [[ -n "$CLEAN_TAG" ]]; then
  echo "All version files match git tag ${EXPECTED_TAG} (${CARGO_VER})."
else
  echo "All version files are consistent (${CARGO_VER})."
fi
