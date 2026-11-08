//! Module preparation: what a wasm module must be for v2 to run it, and the two rewrites v2 applies.
//!
//! Checks (a module failing one is refused, by name where an import is the cause):
//! - it validates under exactly the wasm features NEAR's protocol 74 enables: the MVP with mutable globals
//!   and sign extension (no bulk memory, multi-value, reference types, SIMD, saturating float-to-int, ...);
//! - its size, functions and locals are within nearcore's limits;
//! - a contract imports functions from `env` only, each one a host function v2 implements with nearcore's
//!   semantics, with nearcore's signature; it imports no memory, table or global (as nearcore requires);
//! - a rules module imports functions from `antibody` only (src/rules.rs).
//!
//! Rewrites (byte splicing over wasmparser's offsets; no other change to the module):
//! - **Memory:** as nearcore does, a contract's own memory section is dropped and the module imports
//!   `env.memory` with 1024 pages initial and 2048 maximum, which v2 provides.
//! - **Floats:** every float instruction in a function body (any f32/f64 operator, conversion,
//!   reinterpretation, load, store or constant) becomes `i32.const 1; global.set $flag; unreachable`, where
//!   `$flag` is a new mutable i32 global appended after the module's globals (so no existing index moves)
//!   and exported as [`FLOAT_FLAG_EXPORT`]. A run that reaches one traps with the flag set and is
//!   `Unevaluable`; a module that merely contains float code runs as long as that code is not reached.
//!   Why this and not an imported trap function: an import would shift every defined function's index
//!   (calls, element segments, exports, the start function); a global appended at the end shifts nothing.
//!   wasmi 2.0.0 offers no per-instruction hook; its operator cost table holds at most 255 fuel per
//!   operator, which cannot express "never".
//! - Custom sections are dropped (nearcore discards them too).

use std::ops::Range;

use wasmparser::{Operator, Parser, Payload, TypeRef, ValType, Validator, WasmFeatures};

use crate::config::Limits;

/// The export name of the float flag global. A module that already exports this name is refused.
pub const FLOAT_FLAG_EXPORT: &str = "__antibody_v2_float_executed";

/// The wasm features v2 validates against.
///
/// NEAR protocol 74 enables the MVP plus mutable globals and sign extension, and nothing else (nearcore's
/// contract preparation, version 2). But near-sdk-rs output, linked by recent wasm-ld, encodes the
/// `call_indirect` table immediate as an overlong LEB zero rather than a single `0x00` byte, which
/// wasmparser 0.228 (wasmi's parser) reads only with reference-types parsing on; nearcore's own (older)
/// wasmparser reads it either way, so it accepts these modules and v2 must too. v2 therefore turns on
/// reference-types *parsing* and then refuses, by name, every instruction and construct the reference-types
/// proposal would add on top of the MVP ([`is_refused_op`], the table and element checks in [`prepare`]),
/// so the accepted feature surface stays exactly nearcore's.
pub fn near_features() -> WasmFeatures {
    WasmFeatures::WASM1 | WasmFeatures::SIGN_EXTENSION | WasmFeatures::REFERENCE_TYPES | WasmFeatures::GC_TYPES
}

/// A reference-types (or later) operator nearcore's MVP feature set does not include: refused by name when
/// present, so enabling reference-types *parsing* (above) does not widen what v2 accepts.
pub fn is_refused_op(op: &Operator) -> Option<&'static str> {
    use Operator::*;
    Some(match op {
        RefNull { .. } => "ref.null",
        RefFunc { .. } => "ref.func",
        RefIsNull => "ref.is_null",
        TypedSelect { .. } => "select with a type (reference types)",
        TableGet { .. } => "table.get",
        TableSet { .. } => "table.set",
        TableGrow { .. } => "table.grow",
        TableSize { .. } => "table.size",
        TableFill { .. } => "table.fill",
        TableCopy { .. } => "table.copy",
        TableInit { .. } => "table.init",
        ElemDrop { .. } => "elem.drop",
        ReturnCall { .. } => "return_call (tail calls)",
        ReturnCallIndirect { .. } => "return_call_indirect (tail calls)",
        MemoryCopy { .. } => "memory.copy (bulk memory)",
        MemoryFill { .. } => "memory.fill (bulk memory)",
        MemoryInit { .. } => "memory.init (bulk memory)",
        DataDrop { .. } => "data.drop (bulk memory)",
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A NEAR contract: `env` imports, memory provided by v2.
    Contract,
    /// A program's rules module: `antibody` imports, its own memory.
    Rules,
}

#[derive(Debug, Clone)]
pub struct Prepared {
    pub wasm: Vec<u8>,
    /// Whether the float flag global was added (the module has float instructions in function bodies).
    pub float_flag: bool,
    /// The functions the module imports, by name.
    pub imports: Vec<String>,
    pub has_start: bool,
}

/// A host import: its name and its wasm signature.
pub struct HostSig {
    pub name: &'static str,
    pub params: &'static [ValType],
    pub results: &'static [ValType],
}

const I: ValType = ValType::I64;
const W: ValType = ValType::I32;

macro_rules! sig {
    ($name:literal, [$($p:expr),*], [$($r:expr),*]) => {
        HostSig { name: $name, params: &[$($p),*], results: &[$($r),*] }
    };
}

/// Every NEAR host function v2 implements, with nearcore's signature (docs/near-targets.md §4).
pub const NEAR_HOST_FUNCTIONS: &[HostSig] = &[
    sig!("read_register", [I, I], []),
    sig!("register_len", [I], [I]),
    sig!("write_register", [I, I, I], []),
    sig!("current_account_id", [I], []),
    sig!("signer_account_id", [I], []),
    sig!("signer_account_pk", [I], []),
    sig!("predecessor_account_id", [I], []),
    sig!("input", [I], []),
    sig!("block_index", [], [I]),
    sig!("block_timestamp", [], [I]),
    sig!("epoch_height", [], [I]),
    sig!("storage_usage", [], [I]),
    sig!("account_balance", [I], []),
    sig!("account_locked_balance", [I], []),
    sig!("attached_deposit", [I], []),
    sig!("prepaid_gas", [], [I]),
    sig!("used_gas", [], [I]),
    sig!("random_seed", [I], []),
    sig!("sha256", [I, I, I], []),
    sig!("keccak256", [I, I, I], []),
    sig!("keccak512", [I, I, I], []),
    sig!("ripemd160", [I, I, I], []),
    sig!("ecrecover", [I, I, I, I, I, I, I], [I]),
    sig!("ed25519_verify", [I, I, I, I, I, I], [I]),
    sig!("value_return", [I, I], []),
    sig!("panic", [], []),
    sig!("panic_utf8", [I, I], []),
    sig!("log_utf8", [I, I], []),
    sig!("log_utf16", [I, I], []),
    sig!("abort", [W, W, W, W], []),
    sig!("promise_create", [I, I, I, I, I, I, I, I], [I]),
    sig!("promise_then", [I, I, I, I, I, I, I, I, I], [I]),
    sig!("promise_and", [I, I], [I]),
    sig!("promise_batch_create", [I, I], [I]),
    sig!("promise_batch_then", [I, I, I], [I]),
    sig!("promise_batch_action_create_account", [I], []),
    sig!("promise_batch_action_deploy_contract", [I, I, I], []),
    sig!("promise_batch_action_function_call", [I, I, I, I, I, I, I], []),
    sig!("promise_batch_action_function_call_weight", [I, I, I, I, I, I, I, I], []),
    sig!("promise_batch_action_transfer", [I, I], []),
    sig!("promise_batch_action_add_key_with_full_access", [I, I, I, I], []),
    sig!("promise_batch_action_add_key_with_function_call", [I, I, I, I, I, I, I, I, I], []),
    sig!("promise_batch_action_delete_key", [I, I, I], []),
    sig!("promise_batch_action_delete_account", [I, I, I], []),
    sig!("promise_results_count", [], [I]),
    sig!("promise_result", [I, I], [I]),
    sig!("promise_return", [I], []),
    sig!("storage_write", [I, I, I, I, I], [I]),
    sig!("storage_read", [I, I, I], [I]),
    sig!("storage_remove", [I, I, I], [I]),
    sig!("storage_has_key", [I, I], [I]),
    // Deprecated in NEAR: nearcore links them and every call fails with `Deprecated` (implemented so).
    sig!("storage_iter_prefix", [I, I], [I]),
    sig!("storage_iter_range", [I, I, I, I], [I]),
    sig!("storage_iter_next", [I, I, I], [I]),
    // nearcore's old instrumentation import: charges `opcodes` wasm instructions of gas.
    sig!("gas", [W], []),
];

/// A host function NEAR has and v2 refuses, with its signature and the reason.
pub struct RefusedSig {
    pub name: &'static str,
    pub params: &'static [ValType],
    pub results: &'static [ValType],
    pub reason: &'static str,
}

macro_rules! refused {
    ($name:literal, [$($p:expr),*], [$($r:expr),*], $why:literal) => {
        RefusedSig { name: $name, params: &[$($p),*], results: &[$($r),*], reason: $why }
    };
}

/// Host functions NEAR has and v2 refuses (docs/near-targets.md §4, §13).
///
/// Deviation from the design (see the report and the appended Implementation notes in docs/near-targets.md):
/// the design refuses the *module* that imports one of these. But near-sdk-rs links the whole promise
/// surface, so every near-sdk contract imports `promise_batch_action_stake` whether or not it stakes, and
/// nearcore accepts the module (the import resolves) — only the action is unmodelled. v2 therefore links
/// these to a host function that stops the run *when called*, by name: `Unevaluable` during a run,
/// `ProgramMisconfigured` when it is reached at activation. Importing one, without calling it, is accepted,
/// exactly as nearcore accepts it. This is not a silent stub: a call loudly and deterministically stops the
/// run with the named reason.
pub const REFUSED_HOST_FUNCTIONS: &[RefusedSig] = &[
    refused!("promise_batch_action_stake", [I, I, I, I], [], "v2 models no validators"),
    refused!("validator_stake", [I, I, I], [], "v2 models no validators"),
    refused!("validator_total_stake", [I], [], "v2 models no validators"),
    refused!("promise_yield_create", [I, I, I, I, I, I, I], [I], "v2 models no yielded receipts"),
    refused!("promise_yield_resume", [I, I, I, I], [W], "v2 models no yielded receipts"),
    refused!("alt_bn128_g1_multiexp", [I, I, I], [], "pairing host functions are refused in the first release"),
    refused!("alt_bn128_g1_sum", [I, I, I], [], "pairing host functions are refused in the first release"),
    refused!("alt_bn128_pairing_check", [I, I], [I], "pairing host functions are refused in the first release"),
    refused!("bls12381_p1_sum", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_p2_sum", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_g1_multiexp", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_g2_multiexp", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_map_fp_to_g1", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_map_fp2_to_g2", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_pairing_check", [I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_p1_decompress", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
    refused!("bls12381_p2_decompress", [I, I, I], [I], "BLS12-381 host functions are refused in the first release"),
];

/// The rules module's host functions (src/rules.rs).
pub const RULES_HOST_FUNCTIONS: &[HostSig] = &[
    sig!("view", [I, I, I, I, I, I], [I]),
    sig!("result_len", [], [I]),
    sig!("result_read", [I], []),
    sig!("balance", [I, I, I], [I]),
    sig!("locked_balance", [I, I, I], [I]),
    sig!("ft_balance_of", [I, I, I, I, I], [I]),
    sig!("block_height", [], [I]),
    sig!("block_timestamp", [], [I]),
];

/// Whether an operator is a float instruction (refused when executed).
pub fn is_float_op(op: &Operator) -> bool {
    use Operator::*;
    matches!(
        op,
        F32Load { .. }
            | F64Load { .. }
            | F32Store { .. }
            | F64Store { .. }
            | F32Const { .. }
            | F64Const { .. }
            | F32Eq
            | F32Ne
            | F32Lt
            | F32Gt
            | F32Le
            | F32Ge
            | F64Eq
            | F64Ne
            | F64Lt
            | F64Gt
            | F64Le
            | F64Ge
            | F32Abs
            | F32Neg
            | F32Ceil
            | F32Floor
            | F32Trunc
            | F32Nearest
            | F32Sqrt
            | F32Add
            | F32Sub
            | F32Mul
            | F32Div
            | F32Min
            | F32Max
            | F32Copysign
            | F64Abs
            | F64Neg
            | F64Ceil
            | F64Floor
            | F64Trunc
            | F64Nearest
            | F64Sqrt
            | F64Add
            | F64Sub
            | F64Mul
            | F64Div
            | F64Min
            | F64Max
            | F64Copysign
            | I32TruncF32S
            | I32TruncF32U
            | I32TruncF64S
            | I32TruncF64U
            | I64TruncF32S
            | I64TruncF32U
            | I64TruncF64S
            | I64TruncF64U
            | F32ConvertI32S
            | F32ConvertI32U
            | F32ConvertI64S
            | F32ConvertI64U
            | F32DemoteF64
            | F64ConvertI32S
            | F64ConvertI32U
            | F64ConvertI64S
            | F64ConvertI64U
            | F64PromoteF32
            | I32ReinterpretF32
            | I64ReinterpretF64
            | F32ReinterpretI32
            | F64ReinterpretI64
            | I32TruncSatF32S
            | I32TruncSatF32U
            | I32TruncSatF64S
            | I32TruncSatF64U
            | I64TruncSatF32S
            | I64TruncSatF32U
            | I64TruncSatF64S
            | I64TruncSatF64U
    )
}

fn leb(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn read_leb(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut v = 0u64;
    for (i, &b) in bytes.iter().enumerate().take(10) {
        v |= ((b & 0x7f) as u64) << (7 * i);
        if b & 0x80 == 0 {
            return Some((v, i + 1));
        }
    }
    None
}

fn name(s: &str, out: &mut Vec<u8>) {
    leb(s.len() as u64, out);
    out.extend_from_slice(s.as_bytes());
}

fn section(id: u8, content: &[u8], out: &mut Vec<u8>) {
    out.push(id);
    leb(content.len() as u64, out);
    out.extend_from_slice(content);
}

/// A vector section's content with entries appended: `count + n`, the old entries, the new ones.
fn extend_vec_section(content: &[u8], added: u64, entries: &[u8]) -> Result<Vec<u8>, String> {
    let (count, n) = read_leb(content).ok_or("malformed section count")?;
    let mut out = Vec::with_capacity(content.len() + entries.len() + 5);
    leb(count + added, &mut out);
    out.extend_from_slice(&content[n..]);
    out.extend_from_slice(entries);
    Ok(out)
}

/// The order sections must appear in (custom sections aside).
fn rank(id: u8) -> u8 {
    match id {
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 5,
        13 => 6,
        6 => 7,
        7 => 8,
        8 => 9,
        9 => 10,
        12 => 11,
        10 => 12,
        11 => 13,
        _ => 0,
    }
}

struct Body {
    /// The body's bytes (locals and code), without its size prefix.
    range: Range<usize>,
    /// The byte ranges of its float instructions.
    floats: Vec<Range<usize>>,
}

/// Checks a module and returns the module v2 runs (see the module docs).
pub fn prepare(code: &[u8], kind: Kind, limits: &Limits) -> Result<Prepared, String> {
    if code.len() as u64 > limits.max_contract_size {
        return Err(format!("the module is {} bytes, above NEAR's {} byte limit", code.len(), limits.max_contract_size));
    }
    Validator::new_with_features(near_features())
        .validate_all(code)
        .map_err(|e| format!("the module does not validate under NEAR's wasm features: {} (at byte {})", e.message(), e.offset()))?;

    let (host_fns, import_module): (&[HostSig], &str) = match kind {
        Kind::Contract => (NEAR_HOST_FUNCTIONS, "env"),
        Kind::Rules => (RULES_HOST_FUNCTIONS, "antibody"),
    };
    let mut func_types: Vec<(Vec<ValType>, Vec<ValType>)> = Vec::new();
    let mut sections: Vec<(u8, Range<usize>)> = Vec::new();
    let mut imports = Vec::new();
    let mut imported_funcs = 0u64;
    let mut defined_globals = 0u64;
    let mut exports = Vec::new();
    let mut bodies: Vec<Body> = Vec::new();
    let mut code_section: Option<Range<usize>> = None;
    let mut defined_funcs = 0u64;
    let mut locals = 0u64;
    let mut has_memory = false;
    let mut has_start = false;

    for payload in Parser::new(0).parse_all(code) {
        let payload = payload.map_err(|e| format!("malformed module: {}", e.message()))?;
        match payload {
            Payload::Version { .. } | Payload::End(_) => {}
            Payload::TypeSection(r) => {
                sections.push((1, r.range()));
                for t in r.into_iter_err_on_gc_types() {
                    let t = t.map_err(|e| format!("malformed type: {}", e.message()))?;
                    // nearcore's MVP value types only: a reference type in a signature is refused.
                    if t.params().iter().chain(t.results()).any(|v| matches!(v, ValType::Ref(_) | ValType::V128)) {
                        return Err("a function type uses a reference type, which NEAR does not enable".into());
                    }
                    func_types.push((t.params().to_vec(), t.results().to_vec()));
                }
            }
            Payload::ImportSection(r) => {
                sections.push((2, r.range()));
                for imp in r {
                    let imp = imp.map_err(|e| format!("malformed import: {}", e.message()))?;
                    let ty = match imp.ty {
                        TypeRef::Func(t) => t,
                        TypeRef::Memory(_) => return Err(format!("the module imports memory ({}.{}); NEAR provides it", imp.module, imp.name)),
                        _ => return Err(format!("the module imports a table, global or tag ({}.{})", imp.module, imp.name)),
                    };
                    if imp.module != import_module {
                        return Err(format!("import from module {:?} ({}); only {import_module:?} is allowed", imp.module, imp.name));
                    }
                    let (params, results) = func_types.get(ty as usize).ok_or("import of an unknown type")?;
                    // A refused host function may be imported (near-sdk links the whole surface); calling it
                    // stops the run (src/logic.rs). It is checked by name and signature like any other.
                    if let Some(r) = REFUSED_HOST_FUNCTIONS.iter().find(|r| r.name == imp.name).filter(|_| kind == Kind::Contract) {
                        if params.as_slice() != r.params || results.as_slice() != r.results {
                            return Err(format!("refused host function {} is imported with the wrong signature", imp.name));
                        }
                        imported_funcs += 1;
                        imports.push(imp.name.to_string());
                        continue;
                    }
                    let Some(sig) = host_fns.iter().find(|s| s.name == imp.name) else {
                        return Err(format!("host function {} is not one v2 implements", imp.name));
                    };
                    if params.as_slice() != sig.params || results.as_slice() != sig.results {
                        return Err(format!("host function {} is imported with the wrong signature", imp.name));
                    }
                    imported_funcs += 1;
                    imports.push(imp.name.to_string());
                }
            }
            Payload::FunctionSection(r) => {
                defined_funcs = r.count() as u64;
                sections.push((3, r.range()));
            }
            Payload::TableSection(r) => {
                let range = r.range();
                // nearcore allows the MVP's at most one funcref table.
                if r.count() > 1 {
                    return Err("the module has more than one table (reference types)".into());
                }
                for t in r {
                    let t = t.map_err(|e| format!("malformed table: {}", e.message()))?;
                    if t.ty.element_type != wasmparser::RefType::FUNCREF {
                        return Err("a table is not a funcref table (reference types)".into());
                    }
                }
                sections.push((4, range))
            }
            Payload::MemorySection(r) => {
                has_memory = r.count() > 0;
                sections.push((5, r.range()));
            }
            Payload::GlobalSection(r) => {
                defined_globals = r.count() as u64;
                sections.push((6, r.range()));
            }
            Payload::ExportSection(r) => {
                sections.push((7, r.range()));
                for e in r {
                    exports.push(e.map_err(|e| format!("malformed export: {}", e.message()))?.name.to_string());
                }
            }
            Payload::StartSection { range, .. } => {
                has_start = true;
                sections.push((8, range))
            }
            Payload::ElementSection(r) => sections.push((9, r.range())),
            Payload::DataCountSection { range, .. } => sections.push((12, range)),
            Payload::DataSection(r) => sections.push((11, r.range())),
            Payload::CodeSectionStart { range, .. } => {
                code_section = Some(range.clone());
                sections.push((10, range));
            }
            Payload::CodeSectionEntry(body) => {
                for l in body.get_locals_reader().map_err(|e| e.message().to_string())? {
                    locals += l.map_err(|e| e.message().to_string())?.0 as u64;
                }
                let mut floats = Vec::new();
                let mut ops = body.get_operators_reader().map_err(|e| e.message().to_string())?;
                let mut open: Option<usize> = None;
                while !ops.eof() {
                    let (op, off) = ops.read_with_offset().map_err(|e| e.message().to_string())?;
                    if let Some(why) = is_refused_op(&op) {
                        return Err(format!("the module uses {why}, which NEAR does not enable"));
                    }
                    if let Some(start) = open.take() {
                        floats.push(start..off);
                    }
                    if is_float_op(&op) {
                        open = Some(off);
                    }
                }
                if open.is_some() {
                    return Err("a function body ends with a float instruction".into());
                }
                bodies.push(Body { range: body.range(), floats });
            }
            Payload::CustomSection(_) => {}
            Payload::TagSection(_) | Payload::UnknownSection { .. } => return Err("the module has a section NEAR does not accept".into()),
            _ => return Err("the module is a component, not a core module".into()),
        }
    }
    if imported_funcs + defined_funcs > limits.max_functions_number_per_contract {
        return Err(format!("the module has more than {} functions", limits.max_functions_number_per_contract));
    }
    if locals > limits.max_locals_per_contract {
        return Err(format!("the module declares more than {} locals", limits.max_locals_per_contract));
    }
    if exports.iter().any(|e| e == FLOAT_FLAG_EXPORT) {
        return Err(format!("the module exports the reserved name {FLOAT_FLAG_EXPORT}"));
    }
    if kind == Kind::Rules && !has_memory {
        return Err("a rules module defines and exports its memory".into());
    }

    let float_flag = bodies.iter().any(|b| !b.floats.is_empty());
    let flag_index = defined_globals; // contracts import no globals, rules modules neither
    let mut trap = vec![0x41, 0x01, 0x24];
    leb(flag_index, &mut trap);
    trap.push(0x00);

    // New sections, inserted where the order requires: (id, content).
    let memory_import = {
        let mut e = Vec::new();
        name("env", &mut e);
        name("memory", &mut e);
        e.push(0x02);
        e.push(0x01);
        leb(limits.initial_memory_pages as u64, &mut e);
        leb(limits.max_memory_pages as u64, &mut e);
        e
    };
    let global_entry = vec![0x7f, 0x01, 0x41, 0x00, 0x0b];
    let export_entry = {
        let mut e = Vec::new();
        name(FLOAT_FLAG_EXPORT, &mut e);
        e.push(0x03);
        leb(flag_index, &mut e);
        e
    };
    let has = |id: u8| sections.iter().any(|(i, _)| *i == id);
    let mut pending: Vec<(u8, Vec<u8>)> = Vec::new();
    if kind == Kind::Contract && !has(2) {
        let mut c = vec![1];
        c.extend_from_slice(&memory_import);
        pending.push((2, c));
    }
    if float_flag && !has(6) {
        let mut c = vec![1];
        c.extend_from_slice(&global_entry);
        pending.push((6, c));
    }
    if float_flag && !has(7) {
        let mut c = vec![1];
        c.extend_from_slice(&export_entry);
        pending.push((7, c));
    }

    let mut out = Vec::with_capacity(code.len() + 64);
    out.extend_from_slice(&code[..8]);
    for (id, range) in &sections {
        while let Some(pos) = pending.iter().position(|(p, _)| rank(*p) < rank(*id)) {
            let (p, c) = pending.remove(pos);
            section(p, &c, &mut out);
        }
        let content = &code[range.clone()];
        match id {
            2 if kind == Kind::Contract => section(2, &extend_vec_section(content, 1, &memory_import)?, &mut out),
            5 if kind == Kind::Contract => {} // NEAR provides the memory
            6 if float_flag => section(6, &extend_vec_section(content, 1, &global_entry)?, &mut out),
            7 if float_flag => section(7, &extend_vec_section(content, 1, &export_entry)?, &mut out),
            10 if float_flag => {
                let cs = code_section.clone().unwrap_or_default();
                let (count, _) = read_leb(&code[cs.clone()]).ok_or("malformed code section")?;
                let mut c = Vec::new();
                leb(count, &mut c);
                for b in &bodies {
                    let mut body = Vec::with_capacity(b.range.len() + b.floats.len() * 8);
                    let mut at = b.range.start;
                    for f in &b.floats {
                        body.extend_from_slice(&code[at..f.start]);
                        body.extend_from_slice(&trap);
                        at = f.end;
                    }
                    body.extend_from_slice(&code[at..b.range.end]);
                    leb(body.len() as u64, &mut c);
                    c.extend_from_slice(&body);
                }
                section(10, &c, &mut out);
            }
            _ => section(*id, content, &mut out),
        }
    }
    for (p, c) in pending {
        section(p, &c, &mut out);
    }
    Ok(Prepared { wasm: out, float_flag, imports, has_start })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wasm(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).unwrap()
    }

    #[test]
    fn refuses_by_name() {
        let l = Limits::protocol_74();
        // A refused host function may be imported (calling it stops the run, src/logic.rs), but only with
        // its real signature.
        let m = wasm(r#"(module (import "env" "alt_bn128_pairing_check" (func (param i64 i64) (result i64))) (memory 1) (func (export "f")))"#);
        let p = prepare(&m, Kind::Contract, &l).unwrap();
        assert!(p.imports.iter().any(|i| i == "alt_bn128_pairing_check"));
        let m = wasm(r#"(module (import "env" "alt_bn128_pairing_check" (func (param i64))) (memory 1) (func (export "f")))"#);
        assert!(prepare(&m, Kind::Contract, &l).unwrap_err().contains("wrong signature"));
        let m = wasm(r#"(module (import "env" "no_such_fn" (func)) (func (export "f")))"#);
        assert!(prepare(&m, Kind::Contract, &l).unwrap_err().contains("no_such_fn"));
        let m = wasm(r#"(module (import "env" "sha256" (func (param i32))) (func (export "f")))"#);
        assert!(prepare(&m, Kind::Contract, &l).unwrap_err().contains("wrong signature"));
        let m = wasm(r#"(module (import "other" "sha256" (func (param i64 i64 i64))))"#);
        assert!(prepare(&m, Kind::Contract, &l).is_err());
        let m = wasm(r#"(module (import "env" "memory" (memory 1)))"#);
        assert!(prepare(&m, Kind::Contract, &l).unwrap_err().contains("imports memory"));
    }

    #[test]
    fn refuses_features_near_does_not_enable() {
        let l = Limits::protocol_74();
        let bulk = wasm(r#"(module (memory 1) (func (export "f") (memory.fill (i32.const 0) (i32.const 0) (i32.const 1))))"#);
        assert!(prepare(&bulk, Kind::Contract, &l).unwrap_err().contains("NEAR's wasm features"));
        let multi = wasm(r#"(module (func (export "f") (result i32 i32) (i32.const 1) (i32.const 2)))"#);
        assert!(prepare(&multi, Kind::Contract, &l).is_err());
        let signext = wasm(r#"(module (func (export "f") (result i32) (i32.extend8_s (i32.const 255))))"#);
        assert!(prepare(&signext, Kind::Contract, &l).is_ok(), "sign extension is enabled");
        // Reference-types parsing is on (for the overlong call_indirect encoding near-sdk emits, which the
        // near-sdk integration test exercises), but its instructions and constructs are refused by name.
        let reftypes = wasm(r#"(module (table 1 funcref) (func (export "f") (result funcref) (ref.null func)))"#);
        let e = prepare(&reftypes, Kind::Contract, &l).unwrap_err();
        assert!(e.contains("reference type") || e.contains("ref.null"), "{e}");
        // Bulk memory is refused by validation (the feature is off) before the operator scan runs.
        let fill = wasm(r#"(module (memory 1) (func (export "f") (memory.fill (i32.const 0) (i32.const 0) (i32.const 1))))"#);
        assert!(prepare(&fill, Kind::Contract, &l).unwrap_err().contains("NEAR's wasm features"));
        // A reference-types operator, which validation now accepts, is refused by name by the operator scan.
        let table_get = wasm(r#"(module (table 1 funcref) (func (export "f") (drop (table.get 0 (i32.const 0)))))"#);
        assert!(prepare(&table_get, Kind::Contract, &l).unwrap_err().contains("table.get"));
        let two_tables = wasm(r#"(module (table 1 funcref) (table 1 funcref) (func (export "f")))"#);
        assert!(prepare(&two_tables, Kind::Contract, &l).unwrap_err().contains("more than one table"));
    }

    #[test]
    fn rewrites_floats_and_memory_without_moving_indices() {
        let l = Limits::protocol_74();
        let m = wasm(
            r#"(module
                (import "env" "panic" (func $p))
                (memory (export "memory") 2)
                (global $g (mut i32) (i32.const 7))
                (func $f (export "floaty") (result i32) (i32.trunc_f64_s (f64.add (f64.const 1) (f64.const 2))))
                (func (export "plain") (result i32) (call $f) (drop) (global.get $g)))"#,
        );
        let p = prepare(&m, Kind::Contract, &l).unwrap();
        assert!(p.float_flag);
        assert_eq!(p.imports, vec!["panic".to_string()]);
        // The rewritten module validates, imports memory, and keeps every other index.
        Validator::new_with_features(near_features()).validate_all(&p.wasm).unwrap();
        let mut saw_memory_import = false;
        let mut float_ops = 0;
        for payload in Parser::new(0).parse_all(&p.wasm) {
            match payload.unwrap() {
                Payload::ImportSection(r) => {
                    for i in r {
                        let i = i.unwrap();
                        if let TypeRef::Memory(m) = i.ty {
                            saw_memory_import = i.name == "memory" && m.initial == 1024 && m.maximum == Some(2048);
                        }
                    }
                }
                Payload::MemorySection(r) => assert_eq!(r.count(), 0, "the module's own memory is dropped"),
                Payload::CodeSectionEntry(b) => {
                    let mut ops = b.get_operators_reader().unwrap();
                    while !ops.eof() {
                        if is_float_op(&ops.read().unwrap()) {
                            float_ops += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(saw_memory_import);
        assert_eq!(float_ops, 0, "no float instruction is left");
    }
}
