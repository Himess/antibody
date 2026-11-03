// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET — the Foundry-style practice rules, as the program's slots (docs/foundry.md).

import {PracticeFactory, PracticeVault} from "../../src/practice/Practice.sol";
import {PracticeFoundryInvariantsRules} from "./PracticeFoundryInvariantsRules.sol";

interface Vm {
    function prank(address, address) external;
}

contract PracticeFoundryRulesTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    function test_eachInvariantFunctionIsOneSlot() public {
        PracticeFactory f = new PracticeFactory{value: 24 ether}(24);
        PracticeFoundryInvariantsRules rules = new PracticeFoundryInvariantsRules(f, 1 ether);
        require(rules.check(rules.snapshot()) == 0, "both rules hold at the anchor");
        PracticeVault v = PracticeVault(payable(f.vaultAt(5)));
        vm.prank(address(0xBEEF), address(0xBEEF));
        v.sweep(payable(address(0xBEEF)));
        require(rules.check("") == 1, "slot 0 (vaults keep their ether) breaks, slot 1 holds");
    }
}
