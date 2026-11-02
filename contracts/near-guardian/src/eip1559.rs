//! Minimal RLP + EIP-1559 transaction encoding (no dependencies), shared by the contract and
//! the host-side test binary.

pub fn rlp_bytes(b: &[u8]) -> Vec<u8> {
    if b.len() == 1 && b[0] < 0x80 {
        return b.to_vec();
    }
    let mut out = len_prefix(0x80, 0xb7, b.len());
    out.extend_from_slice(b);
    out
}

pub fn rlp_uint(v: u128) -> Vec<u8> {
    rlp_bytes(&trim(&v.to_be_bytes()))
}

/// Big-endian integer given as bytes (e.g. a 32-byte `r`/`s`), minimally encoded.
pub fn rlp_uint_bytes(be: &[u8]) -> Vec<u8> {
    rlp_bytes(trim(be).as_slice())
}

pub fn rlp_list(items: &[Vec<u8>]) -> Vec<u8> {
    let payload: Vec<u8> = items.concat();
    let mut out = len_prefix(0xc0, 0xf7, payload.len());
    out.extend_from_slice(&payload);
    out
}

fn trim(be: &[u8]) -> Vec<u8> {
    let first = be.iter().position(|&x| x != 0).unwrap_or(be.len());
    be[first..].to_vec()
}

fn len_prefix(short: u8, long: u8, len: usize) -> Vec<u8> {
    if len < 56 {
        vec![short + len as u8]
    } else {
        let lb = trim(&(len as u64).to_be_bytes());
        let mut v = vec![long + lb.len() as u8];
        v.extend_from_slice(&lb);
        v
    }
}

/// Fields of a call-only EIP-1559 transaction with value 0 and an empty access list.
#[derive(Clone, Debug)]
pub struct CallTx {
    pub chain_id: u64,
    pub nonce: u64,
    pub max_priority_fee_per_gas: u128,
    pub max_fee_per_gas: u128,
    pub gas_limit: u64,
    pub to: [u8; 20],
    pub data: Vec<u8>,
}

impl CallTx {
    fn fields(&self) -> Vec<Vec<u8>> {
        vec![
            rlp_uint(self.chain_id as u128),
            rlp_uint(self.nonce as u128),
            rlp_uint(self.max_priority_fee_per_gas),
            rlp_uint(self.max_fee_per_gas),
            rlp_uint(self.gas_limit as u128),
            rlp_bytes(&self.to),
            rlp_uint(0),
            rlp_bytes(&self.data),
            rlp_list(&[]),
        ]
    }

    /// 0x02 || rlp([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList])
    pub fn unsigned(&self) -> Vec<u8> {
        let mut out = vec![0x02];
        out.extend(rlp_list(&self.fields()));
        out
    }

    /// 0x02 || rlp([..., yParity, r, s])
    pub fn signed(&self, y_parity: u8, r: &[u8; 32], s: &[u8; 32]) -> Vec<u8> {
        let mut f = self.fields();
        f.push(rlp_uint(y_parity as u128));
        f.push(rlp_uint_bytes(r));
        f.push(rlp_uint_bytes(s));
        let mut out = vec![0x02];
        out.extend(rlp_list(&f));
        out
    }
}

pub fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.trim_start_matches("0x");
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

pub fn to_hex(b: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(b.len() * 2);
    for x in b {
        s.push(H[(x >> 4) as usize] as char);
        s.push(H[(x & 15) as usize] as char);
    }
    s
}
