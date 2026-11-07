/**
 * EVM addresses a bundle derives (docs/crypto-spec.md §8): the fresh senders of a proof
 * (crates/verifier/src/bundle.rs `sender_address`) and CREATE addresses (alloy's `Address::create`,
 * which crates/witness/src/build.rs `parse_bundle` uses for the created-contract placeholder).
 */
import { keccak_256 } from "@noble/hashes/sha3.js";
import { concat, need, u8, u64le, utf8 } from "./bytes";

/** `keccak256("antibody/sender/v1" ‖ salt ‖ u8(i))[12..]`: sender `i` of a bundle salted with `salt`. */
export function senderAddress(salt: Uint8Array, i: number): Uint8Array {
  need(salt, 32, "sender salt");
  return keccak_256(concat(utf8("antibody/sender/v1"), salt, u8(i))).slice(12);
}

/** RLP of a u64: 0x80 for zero, the byte itself below 0x80, else 0x80 + length ‖ big-endian bytes. */
function rlpU64(n: bigint): Uint8Array {
  const le = u64le(n);
  let len = 8;
  while (len > 0 && le[len - 1] === 0) len--;
  const be = le.slice(0, len).reverse();
  if (len === 0) return u8(0x80);
  if (len === 1 && be[0] < 0x80) return be;
  return concat(u8(0x80 + len), be);
}

/** `keccak256(rlp([sender, nonce]))[12..]`: the contract `sender`'s transaction `nonce` creates. */
export function createAddress(sender: Uint8Array, nonce: bigint): Uint8Array {
  need(sender, 20, "address");
  const body = concat(u8(0x80 + 20), sender, rlpU64(nonce));
  return keccak_256(concat(u8(0xc0 + body.length), body)).slice(12);
}
