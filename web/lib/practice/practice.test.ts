// The browser try against the Rust's vectors: golden/web-vectors.json (crates/cli/tests/web_vectors.rs)
// and golden/vectors.json (crates/cli/tests/golden.rs). Every assertion is byte for byte.
import { readFileSync } from "node:fs";
import { x25519 } from "@noble/curves/ed25519.js";
import { keccak_256 } from "@noble/hashes/sha3.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { describe, expect, it } from "vitest";
import {
  borshString,
  buildPracticeEnvelope,
  checkName,
  CLOSE_TGAS,
  commitment,
  concat,
  createAddress,
  decodePayload,
  encodePayload,
  encodeTx,
  fromHex,
  jsonArgs,
  keyFromSeed,
  openWithKey,
  parseBase,
  parseBundle,
  parseU256,
  parseVaultOf,
  parseVendingConfig,
  practiceContentKey,
  PUBLIC_VEND_SEED,
  publicKeyString,
  publicVendSeed,
  type Rng,
  seal,
  SealError,
  sealPractice,
  senderAddress,
  signTx,
  TGAS,
  toHex,
  toHex0x,
  tutorialBundle,
  u32le,
  unwrapV,
  vaultAddress,
  VEND_TGAS,
  vendArgs,
  wrapV,
  WRAP_V_LEN,
} from "./index";

interface TryVector {
  hunter: string;
  vault_index: number;
  vault: string;
  salt: string;
  bundle: unknown;
  envelope_hex: string;
  envelope_sha256: string;
  content_key: string;
  e: string;
  nonce_w: string;
  nonce_b: string;
  wrap_v: string;
  payload_hex: string;
  blob_hex: string;
  blob_sha256: string;
  commitment: string;
}

interface TxVector {
  args_hex: string;
  block_hash: string;
  deposit: string;
  gas: string;
  method: string;
  nonce: number;
  public_key: string;
  receiver_id: string;
  seed: string;
  signed_base64: string;
  signer_id: string;
  tx_hash: string;
  tx_hex: string;
}

interface WebVectors {
  practice: {
    base: {
      anchor_block: number;
      anchor_hash: string;
      chain_id: number;
      file: string;
      manifest_sha256: string;
      quorum: { block_hash: string; source: string }[];
      sha256: string;
      state_root: string;
    };
    program_key: string;
    round: number;
    sealing_pk: string;
    tries: TryVector[];
  };
  public_vend_key: string;
  public_vend_seed: string;
  transactions: TxVector[];
}

interface GoldenVectors {
  commitment: { blob_sha256: string; commitment: string; hunter: string; program_key: string; round: number }[];
  deployer: { address: string; create_0: string; create_1: string };
  public_vend_key: string;
  sender_address: { address: string; index: number; salt: string }[];
  vend_args: { borsh: string; name: string; public_key: string };
}

const repo = (rel: string) => new URL(`../../../${rel}`, import.meta.url);
const web = JSON.parse(readFileSync(repo("golden/web-vectors.json"), "utf8")) as WebVectors;
const golden = JSON.parse(readFileSync(repo("golden/vectors.json"), "utf8")) as GoldenVectors;
const baseBytes = new Uint8Array(readFileSync(repo(web.practice.base.file)));
const P = web.practice;
const base = parseBase(baseBytes, P.base.sha256);
const sealingPk = fromHex(P.sealing_pk);
const empty = new Uint8Array(0);

/** An Rng handing out fixed byte strings in order (e, nonce_w, nonce_b). */
const fixed = (...chunks: string[]): Rng => {
  const q = chunks.map(fromHex);
  return (n) => {
    const b = q.shift();
    if (b === undefined || b.length !== n) throw new Error(`unexpected draw of ${n} bytes`);
    return b;
  };
};

const flip = (b: Uint8Array, i: number): Uint8Array => {
  const c = b.slice();
  c[i] ^= 1;
  return c;
};

describe("the practice base witness", () => {
  it("parses to the vectors' fields", () => {
    expect(baseBytes.length).toBe(110_956);
    expect(toHex(base.sha256)).toBe(P.base.sha256);
    expect(base.version).toBe(1);
    expect(base.chainId).toBe(BigInt(P.base.chain_id));
    expect(toHex0x(base.claimedBlockHash)).toBe(P.base.anchor_hash);
    expect(base.anchorBlock).toBe(BigInt(P.base.anchor_block));
    expect(toHex0x(base.stateRoot)).toBe(P.base.state_root);
    expect(toHex(base.manifestSha256)).toBe(P.base.manifest_sha256);
    expect(base.quorum.map((q) => ({ source: q.source, block_hash: toHex0x(q.blockHash) }))).toEqual(P.base.quorum);
    expect(toHex(keccak_256(base.headerRlp))).toBe(P.base.anchor_hash.slice(2));
    expect(base.manifest.schema).toBe("antibody.manifest.v2");
    expect(base.invariantsCode.length).toBeGreaterThan(0);
  });

  it("names the vaults of the manifest", () => {
    for (const t of P.tries) expect(vaultAddress(base.manifest, t.vault_index)).toBe(t.vault);
    expect(() => vaultAddress(base.manifest, 24)).toThrow(/no vault-24/);
  });

  it("refuses a base whose hash is not the pinned one", () => {
    expect(() => parseBase(baseBytes, "00".repeat(32))).toThrow(/not the pinned/);
    expect(() => parseBase(flip(baseBytes, baseBytes.length - 1), P.base.sha256)).toThrow(/not the pinned/);
  });

  it("refuses a header that does not hash to the claimed block hash", () => {
    // The claimed hash (offset 20) and a byte of the header's state root.
    expect(() => parseBase(flip(baseBytes, 20))).toThrow(/claimed block hash/);
    const rootAt = base.stateRoot.byteOffset - baseBytes.byteOffset;
    expect(() => parseBase(flip(baseBytes, rootAt))).toThrow(/claimed block hash/);
    expect(() => parseBase(baseBytes.subarray(0, 200))).toThrow();
  });
});

describe.each(P.tries)("the practice try of $hunter (vault $vault_index)", (t) => {
  const envelope = buildPracticeEnvelope(base, t.vault, fromHex(t.salt));
  const k = practiceContentKey(P.program_key, t.hunter);
  const wrap = wrapV(k, sealingPk, P.program_key, fromHex(t.e), fromHex(t.nonce_w));
  const payload = { programKey: P.program_key, hunter: t.hunter, envelope, report: empty };
  const blob = seal(k, payload, wrap, fromHex(t.nonce_b));

  it("builds the tutorial bundle and the submission envelope", () => {
    expect(tutorialBundle(t.vault)).toEqual(t.bundle);
    expect(envelope.length).toBe(363);
    expect(toHex(envelope)).toBe(t.envelope_hex);
    expect(toHex(sha256(envelope))).toBe(t.envelope_sha256);
  });

  it("seals it under the public content key", () => {
    expect(toHex(k)).toBe(t.content_key);
    expect(toHex(wrap)).toBe(t.wrap_v);
    expect(wrap.length).toBe(WRAP_V_LEN);
    expect(toHex(encodePayload(payload))).toBe(t.payload_hex);
    expect(toHex(blob)).toBe(t.blob_hex);
    expect(toHex(sha256(blob))).toBe(t.blob_sha256);
    expect(toHex(commitment(P.program_key, P.round, t.hunter, sha256(blob)))).toBe(t.commitment);
  });

  it("seals the same bytes through sealPractice", () => {
    const s = sealPractice({ sealingPk, programKey: P.program_key, hunter: t.hunter, envelope, rng: fixed(t.e, t.nonce_w, t.nonce_b) });
    expect(toHex(s.blob)).toBe(t.blob_hex);
    expect(toHex(s.sha256)).toBe(t.blob_sha256);
    expect(toHex(s.wrap)).toBe(t.wrap_v);
    expect(toHex(s.contentKey)).toBe(t.content_key);
  });

  it("opens with the content key, and with nothing else", () => {
    expect(openWithKey(blob, k, P.program_key)).toEqual(payload);
    expect(() => openWithKey(blob, k, "bounty.antibody.testnet/7")).toThrow(SealError);
    expect(() => openWithKey(blob, practiceContentKey(P.program_key, "someone-else.testnet"), P.program_key)).toThrow(/authentication failed/);
    expect(() => openWithKey(flip(blob, blob.length - 1), k, P.program_key)).toThrow(/authentication failed/);
    expect(() => openWithKey(flip(blob, 120), k, P.program_key)).toThrow(/authentication failed/);
    expect(() => openWithKey(flip(blob, 0), k, P.program_key)).toThrow(/not an ABSEAL1 blob/);
    expect(() => openWithKey(blob.subarray(0, 127), k, P.program_key)).toThrow(/not an ABSEAL1 blob/);
  });
});

describe("the seal's edges", () => {
  const sk = new Uint8Array(32).fill(42);
  const pkV = x25519.getPublicKey(sk);
  const k = new Uint8Array(32).fill(9);
  const payload = { programKey: "bounty.antibody-hq.testnet/0", hunter: "agent-a.antibody-hq.testnet", envelope: Uint8Array.of(1, 2, 3, 4, 5), report: new TextEncoder().encode("a report") };

  it("unwraps with the program's secret only (sealed.rs round trip)", () => {
    const w = wrapV(k, pkV, payload.programKey, new Uint8Array(32).fill(3), new Uint8Array(12).fill(4));
    expect(unwrapV(w, sk, payload.programKey)).toEqual(k);
    expect(() => unwrapV(w, sk, "bounty.antibody-hq.testnet/1")).toThrow(/authentication failed/);
    expect(() => unwrapV(w, new Uint8Array(32).fill(43), payload.programKey)).toThrow(/authentication failed/);
    const blob = seal(k, payload, w, new Uint8Array(12).fill(5));
    expect(openWithKey(blob, unwrapV(w, sk, payload.programKey), payload.programKey)).toEqual(payload);
  });

  it("refuses non-contributory X25519 points", () => {
    const e = new Uint8Array(32).fill(3);
    const n = new Uint8Array(12);
    expect(() => wrapV(k, new Uint8Array(32), payload.programKey, e, n)).toThrow(/non-contributory/);
    expect(() => wrapV(k, Uint8Array.of(1, ...new Uint8Array(31)), payload.programKey, e, n)).toThrow(/non-contributory/);
    // sealed.rs `low_order_points_are_rejected`: an all-zero epk.
    expect(() => unwrapV(new Uint8Array(WRAP_V_LEN), sk, payload.programKey)).toThrow(/non-contributory/);
  });

  it("refuses a malformed payload", () => {
    const p = encodePayload(payload);
    expect(decodePayload(p)).toEqual(payload);
    expect(() => decodePayload(concat(p, Uint8Array.of(0)))).toThrow(/trailing bytes/);
    expect(() => decodePayload(p.subarray(0, p.length - 1))).toThrow(/truncated/);
    expect(() => decodePayload(flip(p, 0))).toThrow(/payload magic/);
  });
});

describe("the NEAR transactions", () => {
  const practiceKey = keyFromSeed(new Uint8Array(32).fill(0x41));
  const commitArgs = jsonArgs({
    program_id: 6,
    round: 0,
    commitment: P.tries[0].commitment,
    terms_sha256: "2a714747a7fe11e7c534424ec7c9f9b2baca7cb04928aa3aa3b69c32aeffae67",
  });
  const args: Record<string, { args: Uint8Array; tgas: bigint }> = {
    commit: { args: commitArgs, tgas: BigInt(25) },
    vend: { args: vendArgs("web-vector", practiceKey.publicKey), tgas: VEND_TGAS },
    close: { args: jsonArgs({}), tgas: CLOSE_TGAS },
  };

  it.each(web.transactions)("$method: signed as near.rs signs it", (t) => {
    const key = keyFromSeed(fromHex(t.seed));
    expect(publicKeyString(key.publicKey)).toBe(t.public_key);
    const a = args[t.method];
    expect(toHex(a.args)).toBe(t.args_hex);
    expect(a.tgas * TGAS).toBe(BigInt(t.gas));
    const call = { method: t.method, args: a.args, gas: BigInt(t.gas), deposit: BigInt(t.deposit) };
    const tx = encodeTx(t.signer_id, key.publicKey, BigInt(t.nonce), t.receiver_id, t.block_hash, [call]);
    expect(toHex(tx)).toBe(t.tx_hex);
    const s = signTx(tx, key.seed);
    expect(s.hash).toBe(t.tx_hash);
    expect(s.signedBase64).toBe(t.signed_base64);
    expect(s.signed.length).toBe(tx.length + 65);
  });

  it("derives the public vend key from the published seed", () => {
    expect(web.public_vend_seed).toBe(PUBLIC_VEND_SEED);
    expect(toHex(publicVendSeed())).toBe(web.transactions[1].seed);
    expect(publicKeyString(keyFromSeed(publicVendSeed()).publicKey)).toBe(web.public_vend_key);
  });

  it("reads the vending contract's views", () => {
    expect(toHex(borshString("ann"))).toBe("03000000616e6e");
    expect(parseVaultOf(Uint8Array.of(1, 23, 0, 0, 0))).toBe(23);
    expect(parseVaultOf(Uint8Array.of(1, 0, 0, 0, 1, 9))).toBe(2 ** 24);
    expect(parseVaultOf(Uint8Array.of(0))).toBeNull();
    expect(parseVaultOf(Uint8Array.of(1, 4, 0))).toBeNull();
    const config = concat(borshString("antibody.testnet"), borshString("bounty.antibody.testnet"), borshString("wrap.testnet"), u32le(6), Uint8Array.of(7, 7));
    expect(parseVendingConfig(config)).toEqual({ owner: "antibody.testnet", antibody: "bounty.antibody.testnet", token: "wrap.testnet", programId: 6 });
    expect(() => parseVendingConfig(config.subarray(0, config.length - 4))).toThrow(/truncated/);
  });

  it("checks practice names as practice.rs does", () => {
    expect(() => checkName("ann-1_x")).not.toThrow();
    for (const bad of ["", "Ann", "../x", "a.b", "a".repeat(33)]) expect(() => checkName(bad)).toThrow();
  });
});

describe("golden/vectors.json", () => {
  it.each(golden.commitment)("commitment for $hunter", (c) => {
    expect(toHex(commitment(c.program_key, c.round, c.hunter, fromHex(c.blob_sha256)))).toBe(c.commitment);
  });

  it.each(golden.sender_address)("sender $index", (s) => {
    expect(toHex0x(senderAddress(fromHex(s.salt), s.index))).toBe(s.address);
  });

  it("vend args and the public vend key", () => {
    expect(toHex(vendArgs(golden.vend_args.name, fromHex(golden.vend_args.public_key)))).toBe(golden.vend_args.borsh);
    expect(publicKeyString(keyFromSeed(publicVendSeed()).publicKey)).toBe(golden.public_vend_key);
  });

  it("CREATE addresses (the deployer's)", () => {
    const deployer = keccak_256(new TextEncoder().encode("antibody/deployer/v1")).slice(12);
    expect(toHex0x(deployer)).toBe(golden.deployer.address);
    expect(toHex0x(createAddress(deployer, BigInt(0)))).toBe(golden.deployer.create_0);
    expect(toHex0x(createAddress(deployer, BigInt(1)))).toBe(golden.deployer.create_1);
  });
});

describe("parse_bundle", () => {
  const salt = new Uint8Array(32).fill(7);
  const created = `0x${"5c".repeat(19)}00`;
  const call = (to: string | null, data: string) => ({ sender: 0, to, value: "0", data, gas_limit: 1_000_000 });

  it("puts the created contract where the placeholder names it (build.rs test)", () => {
    const b = parseBundle({ senders: 1, endowment_wei: "1", calls: [call(null, "0x6080"), call(created, `0xdeadbeef${"0".repeat(24)}${created.slice(2)}`)] }, salt);
    const want = createAddress(senderAddress(salt, 0), BigInt(0));
    expect(b.calls[1].to).toEqual(want);
    expect(b.calls[1].data.subarray(16)).toEqual(want);
    expect(b.calls[0].to).toBeNull();
    // A call without `to` is a creation too (serde's missing `Option`).
    const noTo = parseBundle({ senders: 1, endowment_wei: "1", calls: [{ sender: 0, value: "0", data: "0x6080", gas_limit: 1 }, call(created, "0x")] }, salt);
    expect(noTo.calls[1].to).toEqual(want);
    expect(() => parseBundle({ senders: 1, endowment_wei: "1", calls: [call(created, "0x")] }, salt)).toThrow(/creates none/);
    expect(() => parseBundle({ senders: 1, endowment_wei: "1", calls: [call(null, "0X6080")] }, salt)).toThrow(/not hex/);
  });

  it("replaces the sender placeholders and parses the amounts as Rust does", () => {
    const b = parseBundle({ senders: 1, endowment_wei: "+5", calls: [{ ...call(null, `0x${"5E".repeat(20)}${"5f".repeat(20)}`), value: "0x1_0" }] }, salt);
    // Sender 1 does not exist: its placeholder stays.
    expect(toHex(b.calls[0].data)).toBe(toHex(senderAddress(salt, 0)) + "5f".repeat(20));
    expect(b.calls[0].value).toBe(BigInt(16));
    expect(b.endowmentWei).toBe(BigInt(5));
    expect(parseU256("1_000")).toBe(BigInt(1000));
    expect(parseU256("0b101")).toBe(BigInt(5));
    expect(parseU256(`0x${"f".repeat(64)}`)).toBe((BigInt(1) << BigInt(256)) - BigInt(1));
    expect(() => parseU256(`0x1${"0".repeat(64)}`)).toThrow(/overflow/);
    expect(() => parseU256("12a")).toThrow(/invalid digit/);
    expect(() => parseBundle({ senders: 1, endowment_wei: "-1", calls: [] }, salt)).toThrow(/u128/);
    expect(() => parseBundle({ senders: 1, endowment_wei: "1", calls: [{ ...call(null, "0x"), sender: 1 }] }, salt)).toThrow(/sender 1 of 1/);
  });
});
