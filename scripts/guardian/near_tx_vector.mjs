// Reference vector for crates/cli/src/near.rs: builds and signs the same transaction with
// near-api-js and prints the sha256 of the borsh transaction and the signed transaction (hex).
//   cd scripts/guardian && npm ci && node near_tx_vector.mjs
import { transactions, utils } from 'near-api-js';
import { createHash } from 'node:crypto';

const seed = new Uint8Array(32).fill(1);
const { ed25519 } = await import('@noble/curves/ed25519');
const pub = ed25519.getPublicKey(seed);
const secret64 = new Uint8Array([...seed, ...pub]);
const kp = utils.KeyPair.fromString('ed25519:' + utils.serialize.base_encode(secret64));
const pk = kp.getPublicKey();

const ONE_NEAR = 10n ** 24n;
const actions = [
  transactions.functionCall('commit', new TextEncoder().encode('{"program_id":0}'), 30n * 10n ** 12n, 0n),
  transactions.transfer(ONE_NEAR),
  transactions.addKey(
    new utils.PublicKey({ keyType: 0, data: new Uint8Array(32).fill(2) }),
    transactions.functionCallAccessKey('bounty.antibody-hq.testnet', ['commit', 'submit'], ONE_NEAR / 4n),
  ),
];
const tx = transactions.createTransaction('agent-a.testnet', pk, 'bounty.antibody-hq.testnet', 7n, actions, new Uint8Array(32).fill(3));
const bytes = tx.encode();
const hash = createHash('sha256').update(bytes).digest();
const sig = kp.sign(hash);
const signed = new transactions.SignedTransaction({
  transaction: tx,
  signature: new transactions.Signature({ keyType: 0, data: sig.signature }),
});
console.log('HASH=' + Buffer.from(hash).toString('hex'));
console.log('SIGNED=' + Buffer.from(signed.encode()).toString('hex'));
