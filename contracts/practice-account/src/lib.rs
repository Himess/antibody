//! A practice account's own contract (docs/practice.md). The practice key may call only this account:
//! `commit`, `reveal`, `retry`, `allocate` and `pay` go on to near-antibody with the same arguments (the
//! practice account is the hunter there), and `close` tells the vending contract that created it (its
//! parent) that the try is done. The vending contract's `reclaim` then calls `close` here, which deletes
//! the account, its NEAR (what its gas did not burn, and this code's storage stake) back to the vending
//! contract. That last transaction is not the account's own: an account deleted in a transaction it
//! signed would lose the gas that transaction set aside.
//!
//! Written without near-sdk: a few hundred bytes of code, so a vend stakes almost nothing for it.

#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}

mod sys {
    extern "C" {
        pub fn input(register_id: u64);
        pub fn register_len(register_id: u64) -> u64;
        pub fn read_register(register_id: u64, ptr: u64);
        pub fn current_account_id(register_id: u64);
        pub fn predecessor_account_id(register_id: u64);
        pub fn prepaid_gas() -> u64;
        pub fn used_gas() -> u64;
        pub fn panic_utf8(len: u64, ptr: u64) -> !;
        pub fn storage_write(key_len: u64, key_ptr: u64, value_len: u64, value_ptr: u64, register_id: u64) -> u64;
        pub fn storage_read(key_len: u64, key_ptr: u64, register_id: u64) -> u64;
        #[allow(clippy::too_many_arguments)]
        pub fn promise_create(
            account_id_len: u64,
            account_id_ptr: u64,
            function_name_len: u64,
            function_name_ptr: u64,
            arguments_len: u64,
            arguments_ptr: u64,
            amount_ptr: u64,
            gas: u64,
        ) -> u64;
        pub fn promise_return(promise_id: u64);
        pub fn promise_batch_create(account_id_len: u64, account_id_ptr: u64) -> u64;
        pub fn promise_batch_action_delete_account(promise_index: u64, beneficiary_id_len: u64, beneficiary_id_ptr: u64);
    }
}

/// Storage key of the near-antibody account calls go on to (set once, by `init`).
const ANTIBODY: &[u8] = b"a";
/// The gas this receipt keeps to create the call it passes on, and to finish: the receipt's and the
/// function call's fees (0.2 + 0.78 Tgas each way, protocol 66) and, per byte of method name and
/// arguments, 47.7 Mgas to send to another account and 2.2 to run (protocol 69).
const KEEP_GAS: u64 = 2_000_000_000_000;
const KEEP_GAS_PER_BYTE: u64 = 60_000_000;
const MAX_ACCOUNT: usize = 64;

/// The arguments of the call passed on (an inline reveal is about 32 KB).
static mut ARGS: [u8; 1 << 16] = [0; 1 << 16];

fn fail(msg: &str) -> ! {
    unsafe { sys::panic_utf8(msg.len() as u64, msg.as_ptr() as u64) }
}

/// Register `r` into `buf`: its length (0 when the register is empty).
fn read(r: u64, buf: &mut [u8]) -> usize {
    let n = unsafe { sys::register_len(r) };
    if n == u64::MAX {
        return 0;
    }
    if n as usize > buf.len() {
        fail("too long");
    }
    unsafe { sys::read_register(r, buf.as_mut_ptr() as u64) };
    n as usize
}

/// This account and the caller.
fn accounts(me: &mut [u8; MAX_ACCOUNT], caller: &mut [u8; MAX_ACCOUNT]) -> (usize, usize) {
    unsafe { sys::current_account_id(0) };
    let m = read(0, me);
    unsafe { sys::predecessor_account_id(0) };
    (m, read(0, caller))
}

/// The vending contract: this account's parent (`<name>.<vending>`).
fn parent(me: &[u8]) -> &[u8] {
    match me.iter().position(|&b| b == b'.') {
        Some(i) => &me[i + 1..],
        None => fail("a practice account has a parent"),
    }
}

/// Calls `method` on `account` with `args`, no deposit and all the gas but what this receipt keeps
/// ([`KEEP_GAS`], and more per byte), and returns its result as this call's.
fn call(account: &[u8], method: &str, args: &[u8]) {
    let keep = KEEP_GAS + (method.len() + args.len()) as u64 * KEEP_GAS_PER_BYTE;
    let gas = unsafe { sys::prepaid_gas().saturating_sub(sys::used_gas()) }.saturating_sub(keep);
    let deposit = 0u128;
    unsafe {
        let p = sys::promise_create(
            account.len() as u64,
            account.as_ptr() as u64,
            method.len() as u64,
            method.as_ptr() as u64,
            args.len() as u64,
            args.as_ptr() as u64,
            &deposit as *const u128 as u64,
            gas,
        );
        sys::promise_return(p);
    }
}

/// Passes this call on to near-antibody, with the same arguments: only this account's key may.
fn forward(method: &str) {
    let (mut me, mut caller) = ([0u8; MAX_ACCOUNT], [0u8; MAX_ACCOUNT]);
    let (m, c) = accounts(&mut me, &mut caller);
    if me[..m] != caller[..c] {
        fail("only this account's key");
    }
    if unsafe { sys::storage_read(ANTIBODY.len() as u64, ANTIBODY.as_ptr() as u64, 1) } == 0 {
        fail("not initialised");
    }
    let mut antibody = [0u8; MAX_ACCOUNT];
    let a = read(1, &mut antibody);
    unsafe { sys::input(2) };
    let args = unsafe { &mut *core::ptr::addr_of_mut!(ARGS) };
    let n = read(2, args);
    call(&antibody[..a], method, &args[..n]);
}

#[no_mangle]
pub extern "C" fn commit() {
    forward("commit")
}

#[no_mangle]
pub extern "C" fn reveal() {
    forward("reveal")
}

#[no_mangle]
pub extern "C" fn retry() {
    forward("retry")
}

#[no_mangle]
pub extern "C" fn allocate() {
    forward("allocate")
}

#[no_mangle]
pub extern "C" fn pay() {
    forward("pay")
}

/// Once, by the vending contract in the batch that creates the account: the near-antibody account calls
/// go on to (the input, as its plain bytes).
#[no_mangle]
pub extern "C" fn init() {
    let (mut me, mut caller) = ([0u8; MAX_ACCOUNT], [0u8; MAX_ACCOUNT]);
    let (m, c) = accounts(&mut me, &mut caller);
    if caller[..c] != *parent(&me[..m]) {
        fail("only the vending contract");
    }
    if unsafe { sys::storage_read(ANTIBODY.len() as u64, ANTIBODY.as_ptr() as u64, 1) } == 1 {
        fail("initialised");
    }
    let mut antibody = [0u8; MAX_ACCOUNT];
    unsafe { sys::input(2) };
    let a = read(2, &mut antibody);
    if a == 0 {
        fail("the near-antibody account");
    }
    unsafe { sys::storage_write(ANTIBODY.len() as u64, ANTIBODY.as_ptr() as u64, a as u64, antibody.as_ptr() as u64, 3) };
}

/// By this account's key: the try is done (the vending contract's `done`), so `reclaim` may close the
/// account at once. By the vending contract: deletes this account, its NEAR to the vending contract.
#[no_mangle]
pub extern "C" fn close() {
    let (mut me, mut caller) = ([0u8; MAX_ACCOUNT], [0u8; MAX_ACCOUNT]);
    let (m, c) = accounts(&mut me, &mut caller);
    let vending = parent(&me[..m]);
    if caller[..c] == *vending {
        unsafe {
            let p = sys::promise_batch_create(m as u64, me.as_ptr() as u64);
            sys::promise_batch_action_delete_account(p, vending.len() as u64, vending.as_ptr() as u64);
        }
    } else if caller[..c] == me[..m] {
        call(vending, "done", &[]);
    } else {
        fail("only this account's key or its vending contract");
    }
}
