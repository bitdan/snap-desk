#!/usr/bin/env bash
# 生成 macOS 代码签名用的自签名证书（只需执行一次，之后长期使用同一张证书）。
#
# macOS 的“屏幕录制”等隐私授权按代码签名身份记录：ad-hoc 签名每次构建都会变，应用更新后授权就失效；
# 固定的自签名证书让签名身份保持不变。它不能替代 Apple Developer ID：首次打开仍需在系统设置中放行。
#
# 用法：bash scripts/macos-signing-cert.sh <私钥输出目录>
# 产物：
#   resources/macos-signing.pem      公钥证书，提交到仓库（CI 用它把证书设为代码签名可信）
#   <目录>/macos-signing.p12         证书 + 私钥，base64 后存入 secrets.MACOS_SIGNING_CERTIFICATE，不要提交
#   <目录>/macos-signing.password    .p12 密码，存入 secrets.MACOS_SIGNING_CERTIFICATE_PASSWORD
# 证书一旦用于发布就不要再换：换证书等于换签名身份，用户需要重新授权一次。
set -euo pipefail

out="${1:?请指定私钥输出目录（不要放在仓库里）}"
root="$(cd "$(dirname "$0")/.." && pwd)"
name='Snap Desk Self-Signed'
mkdir -p "$out"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

cat > "$work/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
O = Snap Desk
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 7300 \
  -config "$work/cert.cnf" -keyout "$work/key.pem" -out "$root/resources/macos-signing.pem"
# Windows 上的 openssl 输出 CRLF；回车必须一起去掉，否则它会成为 .p12 密码的一部分，CI 中解不开证书。
openssl rand -base64 24 | tr -d '\r\n' > "$out/macos-signing.password"
# macOS 的 security import 读不了 OpenSSL 3 默认的 AES 加密 .p12，改用 3DES + SHA1 MAC。
openssl pkcs12 -export -name "$name" -inkey "$work/key.pem" -in "$root/resources/macos-signing.pem" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 \
  -passout "file:$out/macos-signing.password" -out "$out/macos-signing.p12"

echo "公钥证书：$root/resources/macos-signing.pem（提交到仓库）"
echo "私钥与密码：$out（不要提交）。配置 GitHub secrets："
echo "  base64 -w0 \"$out/macos-signing.p12\" | gh secret set MACOS_SIGNING_CERTIFICATE"
echo "  gh secret set MACOS_SIGNING_CERTIFICATE_PASSWORD < \"$out/macos-signing.password\""
