// Derives the Chain Signatures EVM address for (predecessor, path) two ways and checks they agree:
//  (1) locally: pk = root + SHA3-256("near-mpc-recovery v0.1.0 epsilon derivation:" || predecessor || "," || path) * G
//  (2) on chain: v1.signer-prod.testnet.derived_public_key({path, predecessor, domain_id: 0}) (view call)
// Usage: node derive.mjs <predecessor> <path> [testnet|mainnet]
import { secp256k1 } from '@noble/curves/secp256k1';
import { sha3_256, keccak_256 } from '@noble/hashes/sha3';
import bs58 from 'bs58';

const [predecessor, path, net = 'testnet'] = process.argv.slice(2);
const RPC = net === 'mainnet' ? 'https://rpc.mainnet.fastnear.com' : 'https://rpc.testnet.fastnear.com';
const MPC = net === 'mainnet' ? 'v1.signer' : 'v1.signer-prod.testnet';

async function view(method, args) {
  const body = { jsonrpc: '2.0', id: 1, method: 'query', params: { request_type: 'call_function', finality: 'final',
    account_id: MPC, method_name: method, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') } };
  const r = await fetch(RPC, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
  const j = await r.json();
  if (j.error || j.result?.error) throw new Error(JSON.stringify(j.error || j.result.error));
  return JSON.parse(Buffer.from(j.result.result).toString());
}

function addrFromUncompressed64(xy) {
  return '0x' + Buffer.from(keccak_256(xy)).slice(12).toString('hex');
}

function nearPkToPoint(s) {
  // "secp256k1:<base58 of 64-byte x||y>"
  const raw = bs58.decode(s.split(':')[1]);
  const full = new Uint8Array(65); full[0] = 4; full.set(raw, 1);
  return secp256k1.ProjectivePoint.fromHex(full);
}

const t0 = Date.now();
const root = await view('public_key', { domain_id: 0 });
const tweak = sha3_256(new TextEncoder().encode(`near-mpc-recovery v0.1.0 epsilon derivation:${predecessor},${path}`));
const tweakN = BigInt('0x' + Buffer.from(tweak).toString('hex')) % secp256k1.CURVE.n;
const local = nearPkToPoint(root).add(secp256k1.ProjectivePoint.BASE.multiply(tweakN));
const localAddr = addrFromUncompressed64(local.toRawBytes(false).slice(1));
const onchain = await view('derived_public_key', { path, predecessor, domain_id: 0 });
const onchainAddr = addrFromUncompressed64(nearPkToPoint(onchain).toRawBytes(false).slice(1));
console.log(JSON.stringify({ mpc: MPC, predecessor, path, root_key: root, derived_key_onchain: onchain,
  address_local: localAddr, address_onchain: onchainAddr, match: localAddr === onchainAddr, ms: Date.now() - t0 }, null, 1));
