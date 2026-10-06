#!/usr/bin/env bash
#
# Build a signed apt repository from .deb files.
#
#   APT_SIGNING_KEY="$(cat private-key.asc)" tools/apt-repo.sh <out-dir> <deb>...
#
# Leaves in <out-dir>:
#
#   pool/main/k/kuverta/*.deb             the packages
#   dists/stable/main/binary-<arch>/      Packages and Packages.gz per architecture
#   dists/stable/{Release,InRelease,Release.gpg}
#   kuverta.gpg, kuverta.asc              the public key, binary and armoured
#   kuverta.sources                       what goes in /etc/apt/sources.list.d
#
# The directory is static files only; .github/workflows/pages.yml publishes it
# with the website at https://kuverta.github.io/kuverta/apt. It is made again
# from the releases on every run, so nothing here is kept between runs.
#
# APT_SIGNING_KEY is an armoured private OpenPGP key, without a passphrase or
# with one in APT_SIGNING_PASSPHRASE. Needs apt-ftparchive (apt-utils), gpg
# and dpkg-deb. See website/docs/developers/releasing.md.

set -euo pipefail

URL="https://kuverta.github.io/kuverta/apt"
SUITE=stable
COMPONENT=main

if [[ $# -lt 2 ]]; then
    echo "usage: tools/apt-repo.sh <out-dir> <deb>..." >&2
    exit 1
fi
if [[ -z ${APT_SIGNING_KEY:-} ]]; then
    echo "APT_SIGNING_KEY is not set" >&2
    exit 1
fi
OUT="$1"
shift

rm -rf "$OUT"
mkdir -p "$OUT"
OUT=$(cd "$OUT" && pwd)

# The packages, under the path apt expects: pool/<component>/<initial>/<name>.
arches=()
for deb in "$@"; do
    name=$(dpkg-deb -f "$deb" Package)
    arch=$(dpkg-deb -f "$deb" Architecture)
    dir="$OUT/pool/$COMPONENT/${name:0:1}/$name"
    mkdir -p "$dir"
    cp "$deb" "$dir/"
    if [[ $arch != all && " ${arches[*]-} " != *" $arch "* ]]; then
        arches+=("$arch")
    fi
done
if [[ ${#arches[@]} -eq 0 ]]; then
    echo "no architecture-specific packages among $*" >&2
    exit 1
fi

# One index per architecture. Run from $OUT, so each Filename: in it is the
# path relative to the repository's root, which is what apt joins to the URL.
cd "$OUT"
for arch in "${arches[@]}"; do
    dir="dists/$SUITE/$COMPONENT/binary-$arch"
    mkdir -p "$dir"
    apt-ftparchive --arch "$arch" packages pool >"$dir/Packages"
    gzip -9n <"$dir/Packages" >"$dir/Packages.gz"
done

# Release lists every index with its checksums. Written aside first, so it
# does not list itself.
apt-ftparchive \
    -o APT::FTPArchive::Release::Origin=kuverta \
    -o APT::FTPArchive::Release::Label=kuverta \
    -o APT::FTPArchive::Release::Suite="$SUITE" \
    -o APT::FTPArchive::Release::Codename="$SUITE" \
    -o APT::FTPArchive::Release::Components="$COMPONENT" \
    -o APT::FTPArchive::Release::Architectures="${arches[*]}" \
    -o APT::FTPArchive::Release::Description="kuverta, mail and paper post in one place" \
    release "dists/$SUITE" >"$OUT/Release.tmp"
mv "$OUT/Release.tmp" "dists/$SUITE/Release"

# Signed with a keyring of its own that is gone when the script ends, so the
# private key is never left in the runner's ~/.gnupg.
GNUPGHOME=$(mktemp -d)
export GNUPGHOME
trap 'gpgconf --kill gpg-agent 2>/dev/null || true; rm -rf "$GNUPGHOME"' EXIT
gpg --batch --quiet --import <<<"$APT_SIGNING_KEY"
fingerprint=$(gpg --batch --with-colons --list-secret-keys | awk -F: '/^fpr:/ { print $10; exit }')

sign=(gpg --batch --yes --local-user "$fingerprint" --digest-algo SHA512)
if [[ -n ${APT_SIGNING_PASSPHRASE:-} ]]; then
    sign+=(--pinentry-mode loopback --passphrase-fd 3)
fi
"${sign[@]}" --clearsign -o "dists/$SUITE/InRelease" "dists/$SUITE/Release" 3<<<"${APT_SIGNING_PASSPHRASE:-}"
"${sign[@]}" --armor --detach-sign -o "dists/$SUITE/Release.gpg" "dists/$SUITE/Release" 3<<<"${APT_SIGNING_PASSPHRASE:-}"

gpg --batch --export "$fingerprint" >kuverta.gpg
gpg --batch --armor --export "$fingerprint" >kuverta.asc

cat >kuverta.sources <<EOF
Types: deb
URIs: $URL
Suites: $SUITE
Components: $COMPONENT
Signed-By: /usr/share/keyrings/kuverta.gpg
EOF

echo "apt repository for ${arches[*]} in $OUT, signed by $fingerprint"
