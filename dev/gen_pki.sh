#!/usr/bin/env bash
# author: kodeholic (powered by Claude)
# spec: v1.6 · 정:RULE-CHAT-EXT-SUB · 정:IMPL-CFG-SYSTEM-FILE · model: claude-opus-5-5
# 개발 · 2층 형상의 zenoh 입구 시험용 PKI — CA 하나 · hub 서버 인증서(SAN 127.0.0.1 · localhost) · 3rd 클라이언트 인증서(CN = 계정 키) 둘.
# ★시험용이다 — 운영 배포는 제 CA 를 쓴다. 다시 지으려면: bash dev/gen_pki.sh
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p pki
cd pki
rm -f ./*.pem ./*.key ./*.srl ./*.csr
ec() { openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$1"; }
ec ca.key
openssl req -x509 -new -key ca.key -out ca.pem -days 3650 -subj "/CN=oxlens dev zenoh ca" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign"
leaf() {
  local name="$1" cn="$2" ext="$3"
  ec "$name.key"
  openssl req -new -key "$name.key" -out "$name.csr" -subj "/CN=$cn"
  openssl x509 -req -in "$name.csr" -CA ca.pem -CAkey ca.key -CAcreateserial -out "$name.pem" -days 3650 -extfile <(printf '%b' "$ext")
  rm -f "$name.csr"
}
leaf hub hub 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1,DNS:localhost'
leaf ox_3rd ox_3rd 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth'
leaf ox_k_demo ox_k_demo 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth'
leaf ox_hook ox_hook 'basicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=clientAuth'
rm -f ca.srl
ls -1
