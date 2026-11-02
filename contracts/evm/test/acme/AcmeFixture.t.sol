// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// In-EVM checks of the fixture: the anchor state satisfies every invariant, and each scripted
// submission breaks exactly the invariants Antibody's verifier is expected to report. (Root-cause
// fingerprints, receipts and payouts are asserted by the verifier and contract tests.)

import {AcmeSetup} from "../../src/acme/AcmeSetup.sol";
import {AcmeInvariants} from "../../src/acme/AcmeInvariants.sol";
import {IAcmeToken} from "../../src/acme/AcmeTokens.sol";
import {AcmeGuard, GuardPausable} from "../../src/acme/AcmeGuard.sol";
import {
    PocA_SpotPriceBorrow,
    PocC_PoolArbitrage,
    PocD_DonationSyncBorrow,
    PocF_UncheckedRepay,
    PocKnown_DustWithdrawals
} from "./pocs/AcmePocs.sol";

interface Vm {
    function deal(address, uint256) external;
    function prank(address, address) external;
    function prank(address) external;
    function expectRevert(bytes4) external;
}

contract AcmeFixtureTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    AcmeSetup.Deployment d;
    AcmeInvariants inv;
    bytes pre;
    address hunter = address(0xA0A0);
    address guardian = address(0x6A2D);

    function setUp() public {
        d = AcmeSetup.deploy(address(this));
        AcmeSetup.seed(d, address(this));
        d.guard.setPauser(guardian, true);
        inv = new AcmeInvariants(d.lend, d.vault, IAcmeToken(address(d.usd)), address(d.treasury), AcmeSetup.P_REF);
        pre = inv.snapshot();
    }

    function test_anchorSatisfiesAllInvariants() public view {
        require(inv.check(pre) == 0, "anchor must satisfy all invariants");
        require(d.pool.spotPrice() == 2_080e18, "pool spot 4% above reference");
        require(d.vault.accruedFees() == 1_000e18, "fees accrued");
    }

    function test_A_spotPriceBorrow_breaksSolvency() public {
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        new PocA_SpotPriceBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend);
        require(inv.check(pre) == inv.SOLVENCY(), "A: solvency only");
    }

    function test_B_sweepFees_breaksFeesToTreasury() public {
        vm.prank(hunter, hunter);
        d.vault.sweepFees(hunter);
        require(d.usd.balanceOf(hunter) == 1_000e18, "B: took the fees");
        require(inv.check(pre) == inv.FEES_TO_TREASURY(), "B: fees_to_treasury only");
    }

    function test_C_arbitrage_isProfitableButBreaksNothing() public {
        vm.deal(hunter, 10 ether);
        vm.prank(hunter, hunter);
        new PocC_PoolArbitrage{value: 10 ether}(d.eth, d.usd, d.pool, 0.9 ether);
        uint256 usdOut = d.usd.balanceOf(hunter);
        // profit at the reference price: acUSD received - 0.9 acETH * 2,000
        require(usdOut > 1_800e18, "C: profitable at the reference price");
        require(hunter.balance == 9.1 ether, "C: unwrapped the rest");
        require(inv.check(pre) == 0, "C: no invariant broken");
    }

    function test_D_donationSyncBorrow_breaksSolvency() public {
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        new PocD_DonationSyncBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend);
        require(inv.check(pre) == inv.SOLVENCY(), "D: solvency only");
    }

    function test_E_withdrawRounding_breaksSharePrice() public {
        vm.prank(hunter, hunter);
        d.vault.withdraw(1);
        require(d.usd.balanceOf(hunter) == 1, "E: 1 wei for 0 shares");
        require(inv.check(pre) == inv.SHARE_PRICE(), "E: share_price only");
    }

    function test_knownIssue_projectPoC_breaksSharePrice() public {
        vm.prank(address(0x7E57), address(0x7E57));
        new PocKnown_DustWithdrawals(d.vault, 5);
        require(inv.check(pre) == inv.SHARE_PRICE(), "known issue: share_price only");
    }

    function test_F_uncheckedRepay_breaksSolvency() public {
        vm.deal(hunter, 10 ether);
        vm.prank(hunter, hunter);
        new PocF_UncheckedRepay{value: 10 ether}(d.eth, d.usd, d.lend);
        require(hunter.balance == 10 ether, "F: collateral back");
        require(d.usd.balanceOf(hunter) > 0, "F: kept the loan");
        require(inv.check(pre) == inv.SOLVENCY(), "F: solvency only");
    }

    function test_guardianPauseStopsEverything() public {
        vm.prank(guardian);
        d.guard.pause();
        vm.prank(guardian);
        d.guard.pause(); // idempotent
        require(d.lend.paused() && d.vault.paused() && d.pool.paused() && d.usd.paused(), "all paused");
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        vm.expectRevert(GuardPausable.ProtocolPaused.selector);
        d.vault.withdraw(1);
    }

    function test_onlyPauserCanPause() public {
        vm.prank(hunter);
        vm.expectRevert(AcmeGuard.NotPauser.selector);
        d.guard.pause();
    }
}
