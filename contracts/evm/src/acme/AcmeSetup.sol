// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// Deterministic (CREATE2) deployment and seeding of the fictional "Acme Lend" protocol. Shared by
// the Foundry tests and script/DeployAcme.s.sol so tests and testnet start from the same state.

import {AcmeGuard} from "./AcmeGuard.sol";
import {AcmeUSD, AcmeETH, IAcmeToken} from "./AcmeTokens.sol";
import {AcmePool} from "./AcmePool.sol";
import {AcmeLend} from "./AcmeLend.sol";
import {AcmeVault, AcmeTreasury} from "./AcmeVault.sol";
import {AntibodyProxy} from "../antibody/AntibodyAdapter.sol";

/// @notice A seeded borrower position owned by the project (keeps the market realistic).
contract AcmeDemoBorrower {
    address public immutable owner;

    constructor(address owner_) {
        owner = owner_;
    }

    function open(AcmeLend lend, IAcmeToken acETH, uint256 collateral, uint256 debt) external {
        require(msg.sender == owner, "borrower: not owner");
        acETH.approve(address(lend), collateral);
        lend.depositCollateral(collateral);
        lend.borrow(debt);
    }
}

library AcmeSetup {
    bytes32 internal constant SALT = keccak256("antibody/acme-lend/v1");
    /// Committed reference price of acETH (acUSD per acETH, 1e18-scaled).
    uint256 internal constant P_REF = 2_000e18;

    // Seed state (fixture economics; see contracts/evm/README.md)
    uint256 internal constant POOL_ETH = 50e18;
    uint256 internal constant POOL_USD = 104_000e18; // spot 2,080 = 4% above P_REF (room for a small arbitrage)
    uint256 internal constant VAULT_DEPOSIT = 1_000_000e18; // 0.10% fee -> 1,000 acUSD accrued
    uint256 internal constant VAULT_YIELD = 5_000e18; // share price 1,004,000 / 999,000
    uint256 internal constant B1_COLLATERAL = 100e18;
    uint256 internal constant B1_DEBT = 100_000e18;
    uint256 internal constant B2_COLLATERAL = 60e18;
    uint256 internal constant B2_DEBT = 50_000e18;

    struct Deployment {
        AcmeGuard guard;
        AcmeUSD usd;
        AcmeETH eth;
        AcmePool pool;
        /// AcmeLend as users reach it: an AntibodyProxy (a passed release gate upgrades it).
        AcmeLend lend;
        /// The implementation behind the proxy.
        AcmeLend lendImpl;
        AcmeTreasury treasury;
        AcmeVault vault;
        AcmeDemoBorrower borrower1;
        AcmeDemoBorrower borrower2;
    }

    function deploy(address admin) internal returns (Deployment memory d) {
        d.guard = new AcmeGuard{salt: SALT}(admin);
        d.usd = new AcmeUSD{salt: SALT}(d.guard, admin);
        d.eth = new AcmeETH{salt: SALT}(d.guard, admin);
        d.pool = new AcmePool{salt: SALT}(d.guard, IAcmeToken(address(d.eth)), IAcmeToken(address(d.usd)), admin);
        d.lendImpl = new AcmeLend{salt: SALT}(d.guard, IAcmeToken(address(d.usd)), IAcmeToken(address(d.eth)), d.pool);
        // AcmeLend's constructor sets immutables only, so the proxy needs no initializer. The deployer is
        // its admin until the program's adapter exists (`changeAdmin`).
        d.lend = AcmeLend(address(new AntibodyProxy{salt: SALT}(address(d.lendImpl), admin, "")));
        d.treasury = new AcmeTreasury{salt: SALT}(admin);
        d.vault = new AcmeVault{salt: SALT}(d.guard, IAcmeToken(address(d.usd)), d.lend, address(d.treasury), admin);
        d.borrower1 = new AcmeDemoBorrower{salt: keccak256(abi.encode(SALT, 1))}(admin);
        d.borrower2 = new AcmeDemoBorrower{salt: keccak256(abi.encode(SALT, 2))}(admin);
    }

    /// @dev Calls must come from `admin` (the minter, pool admin and borrower owner): the test
    /// contract itself, or the script's broadcaster.
    function seed(Deployment memory d, address admin) internal {
        d.usd.mint(admin, POOL_USD + VAULT_DEPOSIT + VAULT_YIELD);
        d.eth.mint(admin, POOL_ETH);
        d.eth.mint(address(d.borrower1), B1_COLLATERAL);
        d.eth.mint(address(d.borrower2), B2_COLLATERAL);

        d.eth.approve(address(d.pool), POOL_ETH);
        d.usd.approve(address(d.pool), POOL_USD);
        d.pool.addLiquidity(POOL_ETH, POOL_USD);

        d.usd.approve(address(d.vault), VAULT_DEPOSIT + VAULT_YIELD);
        d.vault.deposit(VAULT_DEPOSIT);
        d.vault.donateYield(VAULT_YIELD);

        d.borrower1.open(d.lend, IAcmeToken(address(d.eth)), B1_COLLATERAL, B1_DEBT);
        d.borrower2.open(d.lend, IAcmeToken(address(d.eth)), B2_COLLATERAL, B2_DEBT);
    }
}
