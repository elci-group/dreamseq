#!/usr/bin/env bash
set -euo pipefail

# Exercise an immutable Dreamseq upgrade/rollback entirely inside a temporary
# installation root. Customer configuration and anthologies live outside the
# versioned binary directories and must remain byte-identical throughout.
readonly repository="elci-group/dreamseq"
readonly signer_workflow="elci-group/dreamseq/.github/workflows/release.yml"
readonly old_version="1.3.16"
readonly old_commit="04c18cd87001f61a710d608f08975ec1ce7f582b"
readonly new_version="1.3.17"
readonly new_commit="15f1810b0004a2194254451d7f578f9e77393d05"
readonly target="x86_64-unknown-linux-gnu"

for dependency in curl gh sha256sum tar; do
  command -v "$dependency" >/dev/null || {
    printf 'missing required command: %s\n' "$dependency" >&2
    exit 2
  }
done

drill_root="$(mktemp -d -t dreamseq-rollback.XXXXXXXX)"
case "$drill_root" in
  /tmp/dreamseq-rollback.*) ;;
  *) printf 'refusing unsafe temporary path: %s\n' "$drill_root" >&2; exit 2 ;;
esac
cleanup() {
  case "$drill_root" in
    /tmp/dreamseq-rollback.*) rm -rf -- "$drill_root" ;;
  esac
}
trap cleanup EXIT

artifacts="$drill_root/artifacts"
install_root="$drill_root/install"
state_root="$drill_root/state"
mkdir -p "$artifacts" "$install_root/releases" "$state_root/anthologies"
printf '%s\n' '{"allow_remote_analysis":false,"output_dir":"anthologies"}' >"$state_root/config.json"
printf '%s\n' '{"run_id":"rollback-state-fixture","opportunities":[]}' >"$state_root/anthologies/fixture.json"

state_digest() {
  (cd "$state_root" && find . -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | awk '{print $1}')
}

download_release() {
  local tag="v$1"
  local prefix="https://github.com/$repository/releases/download/$tag"
  local archive="dreamseq-$tag-$target.tar.gz"
  local manifest="dreamseq-$target.sha256"
  local sbom="dreamseq-$target.spdx.json"
  local version_dir="$artifacts/$tag"
  mkdir -p "$version_dir"
  curl --fail --silent --show-error --location "$prefix/$archive" --output "$version_dir/$archive"
  curl --fail --silent --show-error --location "$prefix/$manifest" --output "$version_dir/$manifest"
  curl --fail --silent --show-error --location "$prefix/$sbom" --output "$version_dir/$sbom"
  (cd "$version_dir" && sha256sum --check "$manifest" >/dev/null)
}

verify_provenance() {
  local commit="$2" tag="v$1" asset attempt verified
  for asset in "$artifacts/$tag"/*; do
    verified=false
    for attempt in 1 2 3; do
      if gh attestation verify "$asset" \
        --repo "$repository" \
        --signer-workflow "$signer_workflow" \
        --source-ref "refs/tags/$tag" \
        --source-digest "$commit" \
        --deny-self-hosted-runners \
        --format json >/dev/null 2>&1; then
        verified=true
        break
      fi
      sleep "$attempt"
    done
    test "$verified" = true || {
      printf 'provenance verification failed: %s\n' "$asset" >&2
      exit 1
    }
  done
}

install_release() {
  local tag="v$1"
  local archive="$artifacts/$tag/dreamseq-$tag-$target.tar.gz"
  local release_dir="$install_root/releases/$tag"
  mkdir -p "$release_dir"
  tar --extract --gzip --file "$archive" --directory "$release_dir"
  test -x "$release_dir/dreamseq"
  "$release_dir/dreamseq" --help >/dev/null
}

activate_release() {
  local tag="v$1"
  test -x "$install_root/releases/$tag/dreamseq"
  ln -s "releases/$tag" "$install_root/current.next"
  mv -Tf "$install_root/current.next" "$install_root/current"
  test "$(readlink "$install_root/current")" = "releases/$tag"
}

download_release "$old_version"
download_release "$new_version"
verify_provenance "$old_version" "$old_commit"
verify_provenance "$new_version" "$new_commit"
install_release "$old_version"
install_release "$new_version"
before_digest="$(state_digest)"
activate_release "$old_version"
test "$(state_digest)" = "$before_digest"
activate_release "$new_version"
test "$("$install_root/current/dreamseq" --version)" = "dreamseq $new_version"
test "$(state_digest)" = "$before_digest"
activate_release "$old_version"
"$install_root/current/dreamseq" --help >/dev/null
test "$(state_digest)" = "$before_digest"
activate_release "$new_version"
test "$("$install_root/current/dreamseq" --version)" = "dreamseq $new_version"
after_digest="$(state_digest)"
test "$after_digest" = "$before_digest"

old_archive="dreamseq-v$old_version-$target.tar.gz"
new_archive="dreamseq-v$new_version-$target.tar.gz"
old_archive_digest="$(sha256sum "$artifacts/v$old_version/$old_archive" | awk '{print $1}')"
new_archive_digest="$(sha256sum "$artifacts/v$new_version/$new_archive" | awk '{print $1}')"
printf '{"ok":true,"repository":"%s","target":"%s","sequence":["v%s","v%s","v%s","v%s"],"final_version":"dreamseq %s","state_sha256":"%s","state_preserved":true,"checksums_verified":true,"provenance_verified":true,"old_archive_sha256":"%s","new_archive_sha256":"%s"}\n' \
  "$repository" "$target" "$old_version" "$new_version" "$old_version" "$new_version" \
  "$new_version" "$after_digest" "$old_archive_digest" "$new_archive_digest"
