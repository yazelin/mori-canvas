#!/usr/bin/env bash
# One-shot setup for the host machine:
#   1. install JS deps
#   2. detect this machine's LAN IP
#   3. generate a self-signed cert that includes that IP (so phones can connect over HTTPS)
#
# Run once:  bash setup.sh   (or: npm run setup)
set -e
cd "$(dirname "$0")"

echo "[setup] npm install…"
npm install --no-fund --no-audit

# --- detect LAN IPv4 (prefer the default-route source address) ---
IP="$(ip route get 1.1.1.1 2>/dev/null | grep -oP 'src \K[0-9.]+' | head -1 || true)"
if [ -z "$IP" ]; then
	IP="$(hostname -I 2>/dev/null | tr ' ' '\n' \
		| grep -E '^(192\.168|10\.|172\.(1[6-9]|2[0-9]|3[01]))\.' \
		| grep -vE '^172\.1[78]\.' | head -1 || true)"
fi
echo "[setup] LAN IP = ${IP:-<not found — set it manually in the cert>}"

# --- self-signed cert with localhost + this machine's IP in the SAN ---
mkdir -p certs
SAN="DNS:localhost,IP:127.0.0.1"
[ -n "$IP" ] && SAN="$SAN,IP:$IP"
echo "[setup] generating certs/ (SAN: $SAN)…"
openssl req -x509 -newkey rsa:2048 -nodes \
	-keyout certs/key.pem -out certs/cert.pem -days 825 \
	-subj "/CN=foss-whiteboard-spike" \
	-addext "subjectAltName=$SAN" 2>/dev/null

echo ""
echo "[setup] done. Also make sure ~/.mori/config.json has providers.groq.api_key (for the AI),"
echo "        and mori-ear is installed if you want voice."
echo ""
echo "  本機自己玩:  npm run dev      → http://localhost:5174"
echo "  公司區網版:  npm run dev:lan  → https://${IP:-<your-ip>}:5174   (手機/同事用這個)"
