// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET — the practice program's rule written Foundry-style (docs/foundry.md):
// `antibody invariants from-foundry` turns this contract's `invariant_*` functions into the program's
// slots (PracticeFoundryInvariantsRules.sol, generated next to it).

import {PracticeFactory} from "../../src/practice/Practice.sol";

contract PracticeFoundryInvariants {
    PracticeFactory public immutable factory;
    /// Each vault's ether at the anchor.
    uint256 public immutable each;

    constructor(PracticeFactory factory_, uint256 each_) {
        factory = factory_;
        each = each_;
    }

    function invariant_vaultsKeepTheirEther() public view {
        uint256 n = factory.count();
        for (uint256 i; i < n; ++i) {
            require(factory.vaultAt(i).balance >= each, "a practice vault lost its ether");
        }
    }

    function invariant_factoryHoldsNothing() public view {
        require(address(factory).balance == 0, "the factory holds ether");
    }
}
