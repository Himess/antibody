// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// In-EVM checks of the fixture: the anchor state satisfies every invariant, and each scripted
// submission breaks exactly the invariants Antibody's verifier is expected to report. (Root-cause
// fingerprints, receipts and payouts are asserted by the verifier and contract tests.)

import {AcmeSetup} from "../../src/acme/AcmeSetup.sol";
import {AcmeInvariants, AcmeInvariantsV2} from "../../src/acme/AcmeInvariants.sol";
import {IAcmeToken} from "../../src/acme/AcmeTokens.sol";
import {AcmeGuard, GuardPausable} from "../../src/acme/AcmeGuard.sol";
import {AcmeLendFixed} from "../../src/acme-fixes/AcmeLendFixed.sol";
import {AcmeLendFinal, AcmeVaultFixed} from "../../src/acme-fixes/AcmeFinalFix.sol";
import {AntibodyAdapter, AntibodyProxy} from "../../src/antibody/AntibodyAdapter.sol";
import {
    PocA_SpotPriceBorrow,
    PocC_PoolArbitrage,
    PocD_DonationSyncBorrow,
    PocF_UncheckedRepay,
    PocG_ArbitraryFrom,
    PocKnown_DustWithdrawals
} from "./pocs/AcmePocs.sol";

interface Vm {
    function deal(address, uint256) external;
    function prank(address, address) external;
    function prank(address) external;
    function expectRevert(bytes4) external;
    function etch(address, bytes calldata) external;
    function addr(uint256) external pure returns (address);
    function sign(uint256, bytes32) external pure returns (uint8, bytes32, bytes32);
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

    /// The gap lane: G takes the saver's approved acUSD. Nothing committed breaks; the saver loses it all.
    function test_G_arbitraryFrom_harmsTheSaverAndBreaksNoInvariant() public {
        uint256 before = d.usd.balanceOf(address(d.saver));
        vm.prank(hunter, hunter);
        new PocG_ArbitraryFrom(d.vault, IAcmeToken(address(d.usd)), address(d.saver));
        require(d.usd.balanceOf(address(d.saver)) == 0 && before == AcmeSetup.SAVER_USD, "G: the saver lost everything");
        require(d.vault.balanceOf(hunter) == 0, "G: the PoC contract holds the saver's shares");
        require(inv.check(pre) == 0, "G breaks no committed invariant");
    }

    /// The invariants round 1 adds catch it: bit 3, and only bit 3.
    function test_G_isCaughtByTheInvariantARoundAdds() public {
        AcmeInvariantsV2 inv2 = new AcmeInvariantsV2(d.lend, d.vault, IAcmeToken(address(d.usd)), address(d.treasury), AcmeSetup.P_REF, address(d.saver));
        bytes memory pre2 = inv2.snapshot();
        require(inv2.check(pre2) == 0, "the new invariants hold at the anchor");
        vm.prank(hunter, hunter);
        new PocG_ArbitraryFrom(d.vault, IAcmeToken(address(d.usd)), address(d.saver));
        require(inv2.check(pre2) == inv2.PROTECTED_BALANCES(), "G breaks protected_balances only");
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

    // ---- round 1: the project's fix for bug #1 as a code override at the implementation's address,
    // which is what the verifier judges a round under (the proxy keeps its storage) ----

    function _fixedLend() internal returns (AcmeLendFixed) {
        return new AcmeLendFixed(d.guard, IAcmeToken(address(d.usd)), IAcmeToken(address(d.eth)), d.pool, AcmeSetup.P_REF);
    }

    function _installRound1Fix() internal {
        vm.etch(address(d.lendImpl), address(_fixedLend()).code);
        require(d.lend.collateralPrice() == AcmeSetup.P_REF, "fix: spot 2,080 is capped at the reference");
    }

    /// The same fix shipped the way a passed release gate ships it: a release permit, signed by the
    /// program's guardian key, upgrades the proxy through the adapter to AcmeLendFixed's exact code.
    function test_round1_release_permit_upgrades_the_proxy_and_stops_A() public {
        uint256 guardianKey = 0xA11CE;
        bytes32 program = keccak256("antibody.testnet/0");
        address[] memory targets = new address[](1);
        targets[0] = address(d.guard);
        address[] memory proxies = new address[](1);
        proxies[0] = address(d.lend);
        AntibodyAdapter adapter = new AntibodyAdapter(vm.addr(guardianKey), program, targets, proxies);
        AntibodyProxy(payable(address(d.lend))).changeAdmin(address(adapter));
        AcmeLendFixed fixedLend = _fixedLend();
        AntibodyAdapter.ReleasePermit memory p = AntibodyAdapter.ReleasePermit({
            program: program,
            round: 1,
            proxy: address(d.lend),
            codeHash: address(fixedLend).codehash,
            deadline: block.timestamp + 600
        });
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(guardianKey, adapter.releaseDigest(p));
        adapter.release(p, address(fixedLend), v, r, s);
        require(d.lend.collateralPrice() == AcmeSetup.P_REF, "the proxy runs the fix");
        require(inv.check(pre) == 0, "the upgrade keeps every invariant at the anchor state");
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        try new PocA_SpotPriceBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend) {} catch {}
        require(inv.check(pre) == 0, "A no longer breaks solvency");
        // Nobody else can upgrade it any more, not even the deployer.
        vm.expectRevert(AntibodyProxy.NotAdmin.selector);
        AntibodyProxy(payable(address(d.lend))).upgradeTo(address(d.lendImpl));
    }

    function test_round1_fix_keepsAnchorInvariants() public {
        _installRound1Fix();
        require(inv.check(pre) == 0, "the fix changes no state");
    }

    function test_round1_fix_stopsA() public {
        _installRound1Fix();
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        try new PocA_SpotPriceBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend) {} catch {}
        require(inv.check(pre) == 0, "A no longer breaks solvency");
    }

    function test_round1_fix_stopsD() public {
        _installRound1Fix();
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        try new PocD_DonationSyncBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend) {} catch {}
        require(inv.check(pre) == 0, "D no longer breaks solvency");
    }

    function test_round1_fix_leavesOtherBugs() public {
        _installRound1Fix();
        vm.deal(hunter, 10 ether);
        vm.prank(hunter, hunter);
        new PocF_UncheckedRepay{value: 10 ether}(d.eth, d.usd, d.lend);
        require(inv.check(pre) == inv.SOLVENCY(), "F still breaks solvency (repay is unchanged)");
        vm.prank(address(0x7E57), address(0x7E57));
        new PocKnown_DustWithdrawals(d.vault, 5);
        vm.prank(hunter, hunter);
        d.vault.sweepFees(hunter);
        require(
            inv.check(pre) == inv.SOLVENCY() | inv.SHARE_PRICE() | inv.FEES_TO_TREASURY(),
            "B and the known issue still break their invariants"
        );
    }

    // ------------------------------------------------------------ the final fix (round 2)

    function _finalLend() internal returns (AcmeLendFinal) {
        return new AcmeLendFinal(d.guard, IAcmeToken(address(d.usd)), IAcmeToken(address(d.eth)), d.pool, AcmeSetup.P_REF);
    }

    function _fixedVault() internal returns (AcmeVaultFixed) {
        return new AcmeVaultFixed(d.guard, IAcmeToken(address(d.usd)), d.lend, address(d.treasury), address(this));
    }

    /// The round-2 code overrides: both implementations replaced at their own addresses.
    function _installFinalFix() internal {
        vm.etch(address(d.lendImpl), address(_finalLend()).code);
        vm.etch(address(d.vaultImpl), address(_fixedVault()).code);
    }

    function test_final_fix_keepsAnchorInvariants() public {
        _installFinalFix();
        require(inv.check(pre) == 0, "the fix changes no state");
    }

    function test_final_fix_stopsA() public {
        _installFinalFix();
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        try new PocA_SpotPriceBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend) {} catch {}
        require(inv.check(pre) == 0, "A no longer breaks solvency");
    }

    function test_final_fix_stopsB() public {
        _installFinalFix();
        vm.prank(hunter, hunter);
        vm.expectRevert(AcmeVaultFixed.NotAdmin.selector);
        d.vault.sweepFees(hunter);
        require(inv.check(pre) == 0, "B no longer breaks fees_to_treasury");
    }

    function test_final_fix_stopsD() public {
        _installFinalFix();
        vm.deal(hunter, 100 ether);
        vm.prank(hunter, hunter);
        try new PocD_DonationSyncBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend) {} catch {}
        require(inv.check(pre) == 0, "D no longer breaks solvency");
    }

    function test_final_fix_stopsF() public {
        _installFinalFix();
        vm.deal(hunter, 10 ether);
        vm.prank(hunter, hunter);
        try new PocF_UncheckedRepay{value: 10 ether}(d.eth, d.usd, d.lend) {} catch {}
        require(inv.check(pre) == 0, "F no longer breaks solvency");
    }

    function test_final_fix_stopsG() public {
        _installFinalFix();
        vm.prank(hunter, hunter);
        try new PocG_ArbitraryFrom(d.vault, IAcmeToken(address(d.usd)), address(d.saver)) {} catch {}
        require(d.usd.balanceOf(address(d.saver)) == AcmeSetup.SAVER_USD, "the saver keeps its funds");
    }

    function test_final_fix_leavesTheKnownIssue() public {
        _installFinalFix();
        vm.prank(address(0x7E57), address(0x7E57));
        new PocKnown_DustWithdrawals(d.vault, 5);
        require(inv.check(pre) == inv.SHARE_PRICE(), "the declared known issue is not part of the fix");
    }

    /// The final fix shipped the way a passed release gate ships it: one release permit per proxy, each
    /// to its new implementation's exact code.
    function test_final_release_permits_upgrade_both_proxies() public {
        uint256 guardianKey = 0xA11CE;
        bytes32 program = keccak256("antibody.testnet/0");
        address[] memory targets = new address[](1);
        targets[0] = address(d.guard);
        address[] memory proxies = new address[](2);
        proxies[0] = address(d.lend);
        proxies[1] = address(d.vault);
        AntibodyAdapter adapter = new AntibodyAdapter(vm.addr(guardianKey), program, targets, proxies);
        AntibodyProxy(payable(address(d.lend))).changeAdmin(address(adapter));
        AntibodyProxy(payable(address(d.vault))).changeAdmin(address(adapter));
        address[2] memory impls = [address(_finalLend()), address(_fixedVault())];
        for (uint256 i = 0; i < 2; i++) {
            AntibodyAdapter.ReleasePermit memory p = AntibodyAdapter.ReleasePermit({
                program: program,
                round: 2,
                proxy: proxies[i],
                codeHash: impls[i].codehash,
                deadline: block.timestamp + 600
            });
            (uint8 v, bytes32 r, bytes32 s) = vm.sign(guardianKey, adapter.releaseDigest(p));
            adapter.release(p, impls[i], v, r, s);
        }
        require(d.lend.collateralPrice() == AcmeSetup.P_REF, "AcmeLend runs the final fix");
        require(inv.check(pre) == 0, "the upgrades keep every invariant at the anchor state");
        vm.prank(hunter, hunter);
        vm.expectRevert(AcmeVaultFixed.NotAdmin.selector);
        d.vault.sweepFees(hunter);
    }

    function test_onlyPauserCanPause() public {
        vm.prank(hunter);
        vm.expectRevert(AcmeGuard.NotPauser.selector);
        d.guard.pause();
    }
}
