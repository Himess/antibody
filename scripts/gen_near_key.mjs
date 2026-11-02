// Generate an ed25519 keypair for a NEAR testnet implicit account; prints only the public parts.
import { generateKeyPairSync } from 'node:crypto';
import { writeFileSync, existsSync } from 'node:fs';

const ALPH = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz';
function b58(buf) {
  let n = BigInt('0x' + Buffer.from(buf).toString('hex'));
  let s = '';
  while (n > 0n) { s = ALPH[Number(n % 58n)] + s; n /= 58n; }
  for (const b of buf) { if (b === 0) s = '1' + s; else break; }
  return s;
}

const out = process.argv[2];
if (existsSync(out)) { console.error('exists, refusing to overwrite'); process.exit(1); }
const { publicKey, privateKey } = generateKeyPairSync('ed25519');
const pub = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
const seed = privateKey.export({ format: 'der', type: 'pkcs8' }).subarray(-32);
const accountId = pub.toString('hex');
writeFileSync(out, JSON.stringify({
  account_id: accountId,
  public_key: 'ed25519:' + b58(pub),
  private_key: 'ed25519:' + b58(Buffer.concat([seed, pub])),
}, null, 1));
console.log('NEAR_TESTNET_IMPLICIT_ACCOUNT', accountId);
console.log('NEAR_PUBLIC_KEY', 'ed25519:' + b58(pub));
