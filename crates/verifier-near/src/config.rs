//! The fee schedule and limits v2 reproduces.
//!
//! Source: nearcore's runtime config for protocol version 74 on testnet (the `testnet_74` snapshot of
//! nearcore's parameter store, `near-parameters` 0.29.2). Every value below is copied from that snapshot;
//! the few places where v2 knowingly departs from nearcore's metering are marked `APPROXIMATION`.
//!
//! Gas never decides a verdict (docs/near-targets.md §3): a receipt that runs out of gas, or ends close to
//! it, is `Unevaluable`, and a second run with every wasm and host cost doubled must agree on every
//! observable. The schedule exists so that contracts that read `used_gas` or `prepaid_gas`, or split gas
//! by weight, see plausible numbers, and so the doubled run has something to double.

/// The protocol version whose semantics and fee schedule this build reproduces (`near.protocol`).
pub const PROTOCOL_VERSION: u32 = 74;

/// NEAR testnet's `anchor.chain_id` (docs/near-targets.md §13).
pub const NEAR_TESTNET_CHAIN_ID: u64 = 398;

/// yoctoNEAR per byte of storage (`storage_amount_per_byte`).
pub const STORAGE_AMOUNT_PER_BYTE: u128 = 10_000_000_000_000_000_000;
/// Bytes an account record counts (`storage_num_bytes_account`).
pub const STORAGE_NUM_BYTES_ACCOUNT: u64 = 100;
/// Extra bytes each data record or access key counts (`storage_num_extra_bytes_record`).
pub const STORAGE_NUM_EXTRA_BYTES_RECORD: u64 = 40;
/// Accounts at or below this storage usage need no balance for it (zero-balance accounts, NEP-448).
pub const ZERO_BALANCE_ACCOUNT_STORAGE_LIMIT: u64 = 770;
/// An account whose storage, code excluded, is above this cannot be deleted.
pub const MAX_ACCOUNT_DELETION_STORAGE_USAGE: u64 = 10_000;
/// Top-level accounts shorter than this are created only by the registrar (testnet).
pub const MIN_ALLOWED_TOP_LEVEL_ACCOUNT_LENGTH: usize = 65;
pub const REGISTRAR_ACCOUNT_ID: &str = "registrar";
/// The predecessor of refund receipts.
pub const SYSTEM_ACCOUNT_ID: &str = "system";

/// nearcore's `limit_config` for protocol 74.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    pub max_gas_burnt: u64,
    pub max_total_prepaid_gas: u64,
    pub initial_memory_pages: u32,
    pub max_memory_pages: u32,
    pub registers_memory_limit: u64,
    pub max_register_size: u64,
    pub max_number_registers: u64,
    pub max_number_logs: u64,
    pub max_total_log_length: u64,
    pub max_actions_per_receipt: u64,
    pub max_number_bytes_method_names: u64,
    pub max_length_method_name: u64,
    pub max_arguments_length: u64,
    pub max_length_returned_data: u64,
    pub max_contract_size: u64,
    pub max_length_storage_key: u64,
    pub max_length_storage_value: u64,
    pub max_promises_per_function_call_action: u64,
    pub max_number_input_data_dependencies: u64,
    pub max_functions_number_per_contract: u64,
    pub max_locals_per_contract: u64,
}

impl Limits {
    pub const fn protocol_74() -> Self {
        Limits {
            max_gas_burnt: 300_000_000_000_000,
            max_total_prepaid_gas: 300_000_000_000_000,
            initial_memory_pages: 1_024,
            max_memory_pages: 2_048,
            registers_memory_limit: 1_073_741_824,
            max_register_size: 104_857_600,
            max_number_registers: 100,
            max_number_logs: 100,
            max_total_log_length: 16_384,
            max_actions_per_receipt: 100,
            max_number_bytes_method_names: 2_000,
            max_length_method_name: 256,
            max_arguments_length: 4_194_304,
            max_length_returned_data: 4_194_304,
            max_contract_size: 4_194_304,
            max_length_storage_key: 2_048,
            max_length_storage_value: 4_194_304,
            max_promises_per_function_call_action: 1_024,
            max_number_input_data_dependencies: 128,
            max_functions_number_per_contract: 10_000,
            max_locals_per_contract: 1_000_000,
        }
    }
}

/// Host function costs (`wasm_config.ext_costs`), in gas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtCosts {
    pub base: u64,
    pub contract_loading_base: u64,
    pub contract_loading_bytes: u64,
    pub read_memory_base: u64,
    pub read_memory_byte: u64,
    pub write_memory_base: u64,
    pub write_memory_byte: u64,
    pub read_register_base: u64,
    pub read_register_byte: u64,
    pub write_register_base: u64,
    pub write_register_byte: u64,
    pub utf8_decoding_base: u64,
    pub utf8_decoding_byte: u64,
    pub utf16_decoding_base: u64,
    pub utf16_decoding_byte: u64,
    pub sha256_base: u64,
    pub sha256_byte: u64,
    pub keccak256_base: u64,
    pub keccak256_byte: u64,
    pub keccak512_base: u64,
    pub keccak512_byte: u64,
    pub ripemd160_base: u64,
    pub ripemd160_block: u64,
    pub ed25519_verify_base: u64,
    pub ed25519_verify_byte: u64,
    pub ecrecover_base: u64,
    pub log_base: u64,
    pub log_byte: u64,
    pub storage_write_base: u64,
    pub storage_write_key_byte: u64,
    pub storage_write_value_byte: u64,
    pub storage_write_evicted_byte: u64,
    pub storage_read_base: u64,
    pub storage_read_key_byte: u64,
    pub storage_read_value_byte: u64,
    pub storage_remove_base: u64,
    pub storage_remove_key_byte: u64,
    pub storage_remove_ret_value_byte: u64,
    pub storage_has_key_base: u64,
    pub storage_has_key_byte: u64,
    pub touching_trie_node: u64,
    pub promise_and_base: u64,
    pub promise_and_per_promise: u64,
    pub promise_return: u64,
}

impl ExtCosts {
    pub const fn protocol_74() -> Self {
        ExtCosts {
            base: 264_768_111,
            contract_loading_base: 35_445_963,
            contract_loading_bytes: 1_089_295,
            read_memory_base: 2_609_863_200,
            read_memory_byte: 3_801_333,
            write_memory_base: 2_803_794_861,
            write_memory_byte: 2_723_772,
            read_register_base: 2_517_165_186,
            read_register_byte: 98_562,
            write_register_base: 2_865_522_486,
            write_register_byte: 3_801_564,
            utf8_decoding_base: 3_111_779_061,
            utf8_decoding_byte: 291_580_479,
            utf16_decoding_base: 3_543_313_050,
            utf16_decoding_byte: 163_577_493,
            sha256_base: 4_540_970_250,
            sha256_byte: 24_117_351,
            keccak256_base: 5_879_491_275,
            keccak256_byte: 21_471_105,
            keccak512_base: 5_811_388_236,
            keccak512_byte: 36_649_701,
            ripemd160_base: 853_675_086,
            ripemd160_block: 680_107_584,
            ed25519_verify_base: 210_000_000_000,
            ed25519_verify_byte: 9_000_000,
            ecrecover_base: 278_821_988_457,
            log_base: 3_543_313_050,
            log_byte: 13_198_791,
            storage_write_base: 64_196_736_000,
            storage_write_key_byte: 70_482_867,
            storage_write_value_byte: 31_018_539,
            storage_write_evicted_byte: 32_117_307,
            storage_read_base: 56_356_845_749,
            storage_read_key_byte: 30_952_533,
            storage_read_value_byte: 5_611_004,
            storage_remove_base: 53_473_030_500,
            storage_remove_key_byte: 38_220_384,
            storage_remove_ret_value_byte: 11_531_556,
            storage_has_key_base: 54_039_896_625,
            storage_has_key_byte: 30_790_845,
            touching_trie_node: 16_101_955_926,
            promise_and_base: 1_465_013_400,
            promise_and_per_promise: 5_452_176,
            promise_return: 560_152_386,
        }
    }

    /// Every host function cost multiplied by `k` (the perturbation run uses k = 2).
    pub fn scaled(&self, k: u64) -> Self {
        let m = |v: u64| v.saturating_mul(k);
        ExtCosts {
            base: m(self.base),
            contract_loading_base: m(self.contract_loading_base),
            contract_loading_bytes: m(self.contract_loading_bytes),
            read_memory_base: m(self.read_memory_base),
            read_memory_byte: m(self.read_memory_byte),
            write_memory_base: m(self.write_memory_base),
            write_memory_byte: m(self.write_memory_byte),
            read_register_base: m(self.read_register_base),
            read_register_byte: m(self.read_register_byte),
            write_register_base: m(self.write_register_base),
            write_register_byte: m(self.write_register_byte),
            utf8_decoding_base: m(self.utf8_decoding_base),
            utf8_decoding_byte: m(self.utf8_decoding_byte),
            utf16_decoding_base: m(self.utf16_decoding_base),
            utf16_decoding_byte: m(self.utf16_decoding_byte),
            sha256_base: m(self.sha256_base),
            sha256_byte: m(self.sha256_byte),
            keccak256_base: m(self.keccak256_base),
            keccak256_byte: m(self.keccak256_byte),
            keccak512_base: m(self.keccak512_base),
            keccak512_byte: m(self.keccak512_byte),
            ripemd160_base: m(self.ripemd160_base),
            ripemd160_block: m(self.ripemd160_block),
            ed25519_verify_base: m(self.ed25519_verify_base),
            ed25519_verify_byte: m(self.ed25519_verify_byte),
            ecrecover_base: m(self.ecrecover_base),
            log_base: m(self.log_base),
            log_byte: m(self.log_byte),
            storage_write_base: m(self.storage_write_base),
            storage_write_key_byte: m(self.storage_write_key_byte),
            storage_write_value_byte: m(self.storage_write_value_byte),
            storage_write_evicted_byte: m(self.storage_write_evicted_byte),
            storage_read_base: m(self.storage_read_base),
            storage_read_key_byte: m(self.storage_read_key_byte),
            storage_read_value_byte: m(self.storage_read_value_byte),
            storage_remove_base: m(self.storage_remove_base),
            storage_remove_key_byte: m(self.storage_remove_key_byte),
            storage_remove_ret_value_byte: m(self.storage_remove_ret_value_byte),
            storage_has_key_base: m(self.storage_has_key_base),
            storage_has_key_byte: m(self.storage_has_key_byte),
            touching_trie_node: m(self.touching_trie_node),
            promise_and_base: m(self.promise_and_base),
            promise_and_per_promise: m(self.promise_and_per_promise),
            promise_return: m(self.promise_return),
        }
    }
}

/// An action's fee: burnt when the receipt is created (`send_sir` when the receiver is the current
/// account, `send_not_sir` otherwise) and reserved for its execution (`execution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fee {
    pub send_sir: u64,
    pub send_not_sir: u64,
    pub execution: u64,
}

impl Fee {
    pub const fn new(send_sir: u64, send_not_sir: u64, execution: u64) -> Self {
        Fee { send_sir, send_not_sir, execution }
    }
    pub fn send(&self, sir: bool) -> u64 {
        if sir {
            self.send_sir
        } else {
            self.send_not_sir
        }
    }
}

/// Action fees (`transaction_costs`). The perturbation run keeps these as they are: it doubles wasm and host
/// function costs only (docs/near-targets.md §3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFees {
    pub action_receipt_creation: Fee,
    pub data_receipt_creation_base: Fee,
    pub data_receipt_creation_per_byte: Fee,
    pub create_account: Fee,
    pub deploy_contract: Fee,
    pub deploy_contract_per_byte: Fee,
    pub function_call: Fee,
    pub function_call_per_byte: Fee,
    pub transfer: Fee,
    pub add_full_access_key: Fee,
    pub add_function_call_key: Fee,
    pub add_function_call_key_per_byte: Fee,
    pub delete_key: Fee,
    pub delete_account: Fee,
}

impl ActionFees {
    pub const fn protocol_74() -> Self {
        ActionFees {
            action_receipt_creation: Fee::new(108_059_500_000, 108_059_500_000, 108_059_500_000),
            data_receipt_creation_base: Fee::new(36_486_732_312, 36_486_732_312, 36_486_732_312),
            data_receipt_creation_per_byte: Fee::new(17_212_011, 47_683_715, 17_212_011),
            create_account: Fee::new(3_850_000_000_000, 3_850_000_000_000, 3_850_000_000_000),
            deploy_contract: Fee::new(184_765_750_000, 184_765_750_000, 184_765_750_000),
            deploy_contract_per_byte: Fee::new(6_812_999, 47_683_715, 64_572_944),
            function_call: Fee::new(200_000_000_000, 200_000_000_000, 780_000_000_000),
            function_call_per_byte: Fee::new(2_235_934, 47_683_715, 2_235_934),
            transfer: Fee::new(115_123_062_500, 115_123_062_500, 115_123_062_500),
            add_full_access_key: Fee::new(101_765_125_000, 101_765_125_000, 101_765_125_000),
            add_function_call_key: Fee::new(102_217_625_000, 102_217_625_000, 102_217_625_000),
            add_function_call_key_per_byte: Fee::new(1_925_331, 47_683_715, 1_925_331),
            delete_key: Fee::new(94_946_625_000, 94_946_625_000, 94_946_625_000),
            delete_account: Fee::new(147_489_000_000, 147_489_000_000, 147_489_000_000),
        }
    }
}

/// Everything a run is charged with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub limits: Limits,
    pub ext: ExtCosts,
    pub actions: ActionFees,
    /// Gas per wasm instruction (`regular_op_cost`). v2 counts instructions with wasmi's fuel (one unit per
    /// executed operator, wasmi's default table). APPROXIMATION: nearcore counts with its own instrumentation
    /// and also charges for stack frames; v2 does not.
    pub regular_op_cost: u64,
    /// Fraction of a call's own gas it may use before the run is `Unevaluable`: 9/10 (the 10% margin).
    pub margin_num: u64,
    pub margin_den: u64,
}

impl Config {
    pub fn protocol_74() -> Self {
        Config {
            limits: Limits::protocol_74(),
            ext: ExtCosts::protocol_74(),
            actions: ActionFees::protocol_74(),
            regular_op_cost: 822_756,
            margin_num: 9,
            margin_den: 10,
        }
    }

    /// The perturbation run's schedule: every wasm and host function cost doubled.
    pub fn doubled(&self) -> Self {
        Config { ext: self.ext.scaled(2), regular_op_cost: self.regular_op_cost.saturating_mul(2), ..self.clone() }
    }
}
