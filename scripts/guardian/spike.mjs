// Antibody guardian signing spike (NEAR testnet + Ethereum Sepolia). Testnets only.
//   node spike.mjs status        balances / deployment state
//   node spike.mjs deploy-evm    deploy GuardedVault on Sepolia (pauser = derived guardian address) + fund guardian
//   node spike.mjs deploy-near   deploy + init the guardian contract on the NEAR testnet account
//   node spike.mjs pause         request_pause -> MPC signature -> broadcast -> confirm paused(); timings
//   node spike.mjs failures      failure-mode probes (fee cap, non-owner, duplicate, stale nonce)
// Keys: .secrets/near_guardian.json, .secrets/sepolia_deployer.json (or .env overrides). Never printed.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { connect, keyStores, KeyPair, transactions } from 'near-api-js';

const ROOT = new URL('../../', import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1');
const env = Object.fromEntries((existsSync(ROOT + '.env') ? readFileSync(ROOT + '.env', 'utf8') : '')
  .split(/\r?\n/).filter(l => l && !l.startsWith('#') && l.includes('=')).map(l => [l.slice(0, l.indexOf('=')).trim(), l.slice(l.indexOf('=') + 1).trim()]).filter(([, v]) => v));
const nearKey = JSON.parse(readFileSync(ROOT + '.secrets/near_guardian.json', 'utf8'));
const ACCOUNT = env.NEAR_ACCOUNT_ID || nearKey.account_id;
const NEAR_PRIV = env.NEAR_PRIVATE_KEY || nearKey.private_key;
const EVM_KEY = env.SEPOLIA_DEPLOYER_KEY || JSON.parse(readFileSync(ROOT + '.secrets/sepolia_deployer.json', 'utf8')).private_key;
const SEPOLIA = env.SEPOLIA_RPC || 'https://ethereum-sepolia-rpc.publicnode.com';
const NEAR_RPC = 'https://rpc.testnet.fastnear.com';
const MPC = 'v1.signer-prod.testnet';
const PATH = 'guardian/test-1';
const STATE_FILE = ROOT + 'measurements/guardian_spike_state.json';
const state = existsSync(STATE_FILE) ? JSON.parse(readFileSync(STATE_FILE, 'utf8')) : {};
const save = () => writeFileSync(STATE_FILE, JSON.stringify(state, null, 1));
const cast = (...a) => execFileSync('cast', a, { encoding: 'utf8' }).trim();
const now = () => Date.now();

async function nearRpc(method, params) {
  const r = await fetch(NEAR_RPC, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) });
  const j = await r.json();
  if (j.error) throw new Error(JSON.stringify(j.error));
  return j.result;
}
async function nearView(account_id, method_name, args = {}) {
  const r = await nearRpc('query', { request_type: 'call_function', finality: 'optimistic', account_id, method_name, args_base64: Buffer.from(JSON.stringify(args)).toString('base64') });
  return JSON.parse(Buffer.from(r.result).toString());
}
async function nearAccount() {
  const ks = new keyStores.InMemoryKeyStore();
  await ks.setKey('testnet', ACCOUNT, KeyPair.fromString(NEAR_PRIV));
  const near = await connect({ networkId: 'testnet', keyStore: ks, nodeUrl: NEAR_RPC });
  return near.account(ACCOUNT);
}
function guardianAddress() {
  const out = execFileSync('node', [ROOT + 'scripts/guardian/derive.mjs', ACCOUNT, PATH, 'testnet'], { encoding: 'utf8' });
  const d = JSON.parse(out);
  if (!d.match) throw new Error('local and on-chain derivation disagree');
  return d.address_onchain;
}

async function status() {
  const g = state.guardian_address || guardianAddress();
  state.guardian_address = g; save();
  const deployer = cast('wallet', 'address', '--private-key', EVM_KEY);
  let nearAcct = null;
  try { nearAcct = await nearRpc('query', { request_type: 'view_account', finality: 'final', account_id: ACCOUNT }); } catch (e) { nearAcct = { error: String(e).slice(0, 120) }; }
  console.log(JSON.stringify({
    near_account: ACCOUNT, near_balance_yocto: nearAcct?.amount ?? null, near_code_hash: nearAcct?.code_hash ?? null, near_error: nearAcct?.error,
    sepolia_chain_id: cast('chain-id', '--rpc-url', SEPOLIA), deployer, deployer_balance_wei: cast('balance', deployer, '--rpc-url', SEPOLIA),
    guardian_address: g, guardian_balance_wei: cast('balance', g, '--rpc-url', SEPOLIA), vault: state.vault ?? null,
  }, null, 1));
}

async function deployEvm() {
  if (cast('chain-id', '--rpc-url', SEPOLIA) !== '11155111') throw new Error('not Sepolia');
  const g = state.guardian_address || guardianAddress();
  const out = execFileSync('forge', ['create', 'src/GuardedVault.sol:GuardedVault', '--rpc-url', SEPOLIA, '--private-key', EVM_KEY, '--broadcast', '--constructor-args', g],
    { cwd: ROOT + 'contracts/evm', encoding: 'utf8' });
  state.vault = out.match(/Deployed to: (0x[0-9a-fA-F]{40})/)[1];
  state.vault_deploy_tx = out.match(/Transaction hash: (0x[0-9a-fA-F]{64})/)[1];
  save();
  const fund = cast('send', g, '--value', '0.01ether', '--private-key', EVM_KEY, '--rpc-url', SEPOLIA, '--json');
  state.guardian_fund_tx = JSON.parse(fund).transactionHash; save();
  console.log(JSON.stringify({ vault: state.vault, deploy_tx: state.vault_deploy_tx, guardian: g, fund_tx: state.guardian_fund_tx }, null, 1));
}

async function deployNear() {
  if (!state.vault) throw new Error('deploy-evm first');
  const account = await nearAccount();
  const wasm = readFileSync(ROOT + 'contracts/near-guardian/target/wasm32-unknown-unknown/release/antibody_guardian.wasm');
  const args = {
    owner: ACCOUNT, mpc: MPC, path: PATH, guardian_address: state.guardian_address,
    target: { chain_id: 11155111, to: state.vault, calldata: '0x8456cb59', gas_limit: 100000,
      max_fee_per_gas_cap: '200000000000', max_priority_fee_per_gas_cap: '20000000000' },
  };
  const t0 = now();
  const res = await account.signAndSendTransaction({ receiverId: ACCOUNT, actions: [
    transactions.deployContract(wasm), transactions.functionCall('new', args, 100000000000000n, 0n)] });
  state.near_deploy_tx = res.transaction.hash; save();
  console.log(JSON.stringify({ deploy_tx: res.transaction.hash, ms: now() - t0, config: await nearView(ACCOUNT, 'get_config') }, null, 1));
}

async function feeParams() {
  const base = BigInt(cast('base-fee', '--rpc-url', SEPOLIA));
  const prio = 2_000_000_000n;
  return { maxFee: base * 3n + prio, prio };
}

async function requestPause(nonce, maxFee, prio, gas = 300000000000000n) {
  const account = await nearAccount();
  const before = (await nearView(ACCOUNT, 'get_config')).next_request;
  const t0 = now();
  let outcome = null; let err = null;
  try {
    outcome = await account.functionCall({ contractId: ACCOUNT, methodName: 'request_pause',
      args: { nonce, max_fee_per_gas: maxFee.toString(), max_priority_fee_per_gas: prio.toString() }, gas, attachedDeposit: 0n });
  } catch (e) { err = String(e).slice(0, 300); }
  const tSubmitted = now();
  // Wait for the yield/resume callback to record the signature (or a failure).
  let last = null; let tSigned = null;
  for (let i = 0; i < 240; i++) {
    const l = await nearView(ACCOUNT, 'get_last');
    if (l && l.request_id >= before) { last = l; tSigned = now(); break; }
    const cfg = await nearView(ACCOUNT, 'get_config');
    if (cfg.failures > (state.failures_seen ?? 0)) { state.failures_seen = cfg.failures; save(); break; }
    await new Promise(r => setTimeout(r, 500));
  }
  return { outcome, err, last, t0, tSubmitted, tSigned };
}

async function pause() {
  const g = state.guardian_address;
  const nonce = Number(cast('nonce', g, '--rpc-url', SEPOLIA));
  const { maxFee, prio } = await feeParams();
  const r = await requestPause(nonce, maxFee, prio);
  if (!r.last) throw new Error('no signature recorded: ' + (r.err || 'timeout'));
  const txHash = r.outcome?.transaction?.hash;
  let gasBurnt = null;
  if (txHash) {
    const st = await nearRpc('EXPERIMENTAL_tx_status', { tx_hash: txHash, sender_account_id: ACCOUNT, wait_until: 'FINAL' });
    gasBurnt = st.receipts_outcome.reduce((a, o) => a + BigInt(o.outcome.gas_burnt), BigInt(st.transaction_outcome.outcome.gas_burnt)).toString();
    state.pause_near_tx = txHash;
  }
  const tB = now();
  const pub = JSON.parse(cast('publish', r.last.signed_tx, '--rpc-url', SEPOLIA));
  const ethTx = pub.transactionHash;
  let receipt = null;
  for (let i = 0; i < 120 && !receipt; i++) {
    try { receipt = JSON.parse(cast('receipt', ethTx, '--rpc-url', SEPOLIA, '--json')); } catch { await new Promise(res => setTimeout(res, 1000)); }
  }
  const tIncluded = now();
  const paused = cast('call', state.vault, 'paused()(bool)', '--rpc-url', SEPOLIA);
  const result = {
    near_request_tx: txHash, near_gas_burnt_total: gasBurnt, signed_tx: r.last.signed_tx, recovered_address: r.last.recovered_address,
    requested_at_block: r.last.requested_at_block, signed_at_block: r.last.signed_at_block, near_blocks_to_signature: r.last.signed_at_block - r.last.requested_at_block,
    eth_tx: ethTx, eth_status: receipt?.status, eth_from: cast('tx', ethTx, 'from', '--rpc-url', SEPOLIA), eth_gas_used: receipt?.gasUsed, eth_block: receipt?.blockNumber,
    paused, latency_ms: { request_to_signature_observed: r.tSigned - r.t0, near_tx_submit: r.tSubmitted - r.t0, broadcast_to_inclusion: tIncluded - tB, total: tIncluded - r.t0 },
  };
  state.pause_result = result; save();
  console.log(JSON.stringify(result, null, 1));
}

async function failures() {
  const g = state.guardian_address;
  const { maxFee, prio } = await feeParams();
  const out = {};
  const account = await nearAccount();
  try { await account.functionCall({ contractId: ACCOUNT, methodName: 'request_pause', args: { nonce: 0, max_fee_per_gas: '999999999999999', max_priority_fee_per_gas: '1' }, gas: 100000000000000n, attachedDeposit: 0n }); out.fee_above_cap = 'ACCEPTED (unexpected)'; }
  catch (e) { out.fee_above_cap = 'rejected: ' + (String(e).match(/maxFeePerGas above cap/) ? 'maxFeePerGas above cap' : String(e).slice(0, 120)); }
  // Duplicate pause: a fresh signature for the next nonce; pause() is idempotent on chain.
  const nonce = Number(cast('nonce', g, '--rpc-url', SEPOLIA));
  const r = await requestPause(nonce, maxFee, prio);
  if (r.last) {
    const pub = JSON.parse(cast('publish', r.last.signed_tx, '--rpc-url', SEPOLIA));
    let rc = null; for (let i = 0; i < 120 && !rc; i++) { try { rc = JSON.parse(cast('receipt', pub.transactionHash, '--rpc-url', SEPOLIA, '--json')); } catch { await new Promise(res => setTimeout(res, 1000)); } }
    out.duplicate_pause = { status: rc?.status, pauseCount: cast('call', state.vault, 'pauseCount()(uint256)', '--rpc-url', SEPOLIA), gasUsed: rc?.gasUsed };
    try { cast('publish', r.last.signed_tx, '--rpc-url', SEPOLIA); out.replay_same_raw_tx = 'ACCEPTED (unexpected)'; } catch (e) { out.replay_same_raw_tx = 'rejected (nonce already used)'; }
  }
  state.failures_result = out; save();
  console.log(JSON.stringify(out, null, 1));
}

const cmd = process.argv[2];
const fns = { status, 'deploy-evm': deployEvm, 'deploy-near': deployNear, pause, failures };
if (!fns[cmd]) { console.error('usage: node spike.mjs <status|deploy-evm|deploy-near|pause|failures>'); process.exit(2); }
await fns[cmd]();
