"""Writes fixtures/zk/t2-test-key.json: a TEST Groth16 key for near-t2 in the sandbox and in unit tests.

Its beta, gamma and delta are all the G2 generator, so anyone can make a "proof" for any public inputs:
with B = G2 and any C, A = alpha + L + C satisfies e(A, B) = e(alpha, beta) e(L, gamma) e(C, delta)
(L = IC0 + sum of input_i * IC_(i+1)). That is the point: the sandbox's MOCK prover makes proofs for the
real public values of sandbox runs, so the contract's comparisons (and the lying-TEE case) run end to end
without SP1. It must never be deployed to testnet: there the contract uses SP1 6.10.0's key, built in.

NEAR's encoding: field elements little-endian, G1 as (x, y), G2 as (x.c0, x.c1, y.c0, y.c1).
usage: t2_test_key.py <out.json>
"""
import json, sys

P = 0x30644E72E131A029B85045B68181585D97816A916871CA8D3C208C16D87CFD47
G1 = (1, 2)
G2 = ((0x1800DEEF121F1E76426A00665E5C4479674322D4F75EDADD46DEBD5CD992F6ED, 0x198E9393920D483A7260BFB731FB5D25F1AA493335A9E71297E485B7AEF312C2),
      (0x12C85EA5DB8C6DEB4AAB71808DCB408FE3D1E7690C43D37B4CE6CC0166FA7DAA, 0x090689D0585FF075EC9E99AD690C3395BC4B313370B38EF355ACDADCD122975B))


def add(p, q):
    if p is None: return q
    if q is None: return p
    if p[0] == q[0] and (p[1] + q[1]) % P == 0: return None
    if p == q:
        l = 3 * p[0] * p[0] * pow(2 * p[1], P - 2, P) % P
    else:
        l = (q[1] - p[1]) * pow(q[0] - p[0], P - 2, P) % P
    x = (l * l - p[0] - q[0]) % P
    return (x, (l * (p[0] - x) - p[1]) % P)


def mul(k, p=G1):
    r = None
    while k:
        if k & 1: r = add(r, p)
        p = add(p, p)
        k >>= 1
    return r


le = lambda v: v.to_bytes(32, "little").hex()
g1 = lambda pt: le(pt[0]) + le(pt[1])
neg_g2 = le(G2[0][0]) + le(G2[0][1]) + le((P - G2[1][0]) % P) + le((P - G2[1][1]) % P)

assert mul(2) == add(G1, G1) and (G1[1] ** 2 - G1[0] ** 3 - 3) % P == 0
key = {
    "note": "TEST key (scripts/zk/t2_test_key.py): beta = gamma = delta = the G2 generator, so anyone can make a proof that verifies. Sandbox and unit tests only; never deploy it.",
    "selector": "7e577e57",
    "vk_root": "00" * 31 + "01",
    "alpha": g1(mul(2)),
    "beta_neg": neg_g2,
    "gamma_neg": neg_g2,
    "delta_neg": neg_g2,
    "ic": [g1(mul(3 + i)) for i in range(6)],
}
open(sys.argv[1], "w", newline="\n").write(json.dumps(key, indent=1) + "\n")
print(f"wrote {sys.argv[1]}")
