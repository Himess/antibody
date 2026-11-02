//! Host-side check of the contract's EIP-1559 encoder.
//!   assemble unsigned <chain> <nonce> <prio> <maxfee> <gas> <to> <data>
//!   assemble signed   <chain> <nonce> <prio> <maxfee> <gas> <to> <data> <yParity> <r> <s>
#[path = "../../src/eip1559.rs"]
mod eip1559;
use eip1559::*;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let to_b = hex_to_bytes(&a[7]).expect("to");
    let mut to = [0u8; 20];
    to.copy_from_slice(&to_b);
    let tx = CallTx {
        chain_id: a[2].parse().unwrap(),
        nonce: a[3].parse().unwrap(),
        max_priority_fee_per_gas: a[4].parse().unwrap(),
        max_fee_per_gas: a[5].parse().unwrap(),
        gas_limit: a[6].parse().unwrap(),
        to,
        data: hex_to_bytes(&a[8]).expect("data"),
    };
    match a[1].as_str() {
        "unsigned" => println!("0x{}", to_hex(&tx.unsigned())),
        "signed" => {
            let r: [u8; 32] = hex_to_bytes(&a[10]).unwrap().try_into().unwrap();
            let s: [u8; 32] = hex_to_bytes(&a[11]).unwrap().try_into().unwrap();
            println!("0x{}", to_hex(&tx.signed(a[9].parse().unwrap(), &r, &s)));
        }
        _ => panic!("mode"),
    }
}
