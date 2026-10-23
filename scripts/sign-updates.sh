#!/bin/sh
# Release publishing only. The private key is supplied on stdin, never in arguments.
set -eu
umask 077
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM
cat > "$work/private.pem"
openssl pkey -in "$work/private.pem" -pubout -out "$work/public.pem"
cmp assets/update-public.pem "$work/public.pem"
for manifest in mira-*.update.json; do
  [ -f "$manifest" ] || exit 1
  openssl pkeyutl -sign -rawin -inkey "$work/private.pem" -in "$manifest" -out "$manifest.sig"
  openssl pkeyutl -verify -pubin -rawin -inkey assets/update-public.pem -in "$manifest" -sigfile "$manifest.sig"
done
