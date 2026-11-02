// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// `antibody invariants draft --source contracts/evm/src/acme` output (golden file DraftInvariants.sol,
// kept in sync by crates/cli/src/draft.rs tests). It compiles, holds at the fixture's anchor, and
// catches exactly what it claims to: the share price rule. Solvency is left to the project as a
// template, so the draft must not report it.

import {AcmeSetup} from "../../src/acme/AcmeSetup.sol";
import {DraftInvariants} from "./DraftInvariants.sol";
import {PocA_SpotPriceBorrow, PocKnown_DustWithdrawals} from "../acme/pocs/AcmePocs.sol";

interface Vm {
    function deal(address, uint256) external;
    function prank(address, address) external;
}

contract DraftInvariantsTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    AcmeSetup.Deployment d;
    DraftInvariants inv;
    bytes pre;

    function setUp() public {
        d = AcmeSetup.deploy(address(this));
        AcmeSetup.seed(d, address(this));
        inv = new DraftInvariants(address(d.vault));
        pre = inv.snapshot();
    }

    function test_draftHoldsAtTheAnchor() public view {
        require(inv.check(pre) == 0, "the draft holds on the honest state");
    }

    function test_draftCatchesTheShareRounding() public {
        vm.prank(address(0x7E57), address(0x7E57));
        new PocKnown_DustWithdrawals(d.vault, 5);
        require(inv.check(pre) == inv.SHARE_PRICE_ACMEVAULT(), "share price fell");
    }

    function test_draftDoesNotClaimSolvency() public {
        vm.deal(address(0xA0A0), 100 ether);
        vm.prank(address(0xA0A0), address(0xA0A0));
        new PocA_SpotPriceBorrow{value: 100 ether}(d.eth, d.usd, d.pool, d.lend);
        require(inv.check(pre) == 0, "solvency is a template, not a drafted rule");
    }
}
