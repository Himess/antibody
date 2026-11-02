// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET — the practice program's vaults and their one rule (docs/practice.md).

import {PracticeFactory, PracticeVault, PracticeInvariants} from "../../src/practice/Practice.sol";

interface Vm {
    function prank(address, address) external;
}

contract PracticeTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

    PracticeFactory factory;
    PracticeInvariants inv;
    bytes pre;
    address hunter = address(0xA0A0);

    function setUp() public {
        factory = new PracticeFactory{value: 24 ether}(24);
        inv = new PracticeInvariants(factory);
        pre = inv.snapshot();
    }

    function test_everyVaultIsFundedAtItsComputedAddress() public view {
        for (uint256 i; i < 24; ++i) {
            address v = factory.vaultAt(i);
            require(v.code.length > 0 && v.balance == 1 ether, "a funded vault at the CREATE2 address");
            require(PracticeVault(payable(v)).owner() == address(this), "owned by the deployer");
        }
        require(inv.check(pre) == 0, "the rule holds at the anchor");
    }

    /// The tutorial PoC: one call, sweeping your own vault.
    function test_tutorialPoC_breaksTheRule() public {
        PracticeVault v = PracticeVault(payable(factory.vaultAt(7)));
        vm.prank(hunter, hunter);
        v.sweep(payable(address(0xdEaD)));
        require(address(v).balance == 0, "the vault is empty");
        require(inv.check(pre) == inv.VAULT_KEEPS_ITS_ETHER(), "vault_keeps_its_ether breaks");
    }

    function test_otherVaultsAreUntouched() public {
        vm.prank(hunter, hunter);
        PracticeVault(payable(factory.vaultAt(3))).sweep(payable(address(0xdEaD)));
        for (uint256 i; i < 24; ++i) {
            if (i != 3) require(factory.vaultAt(i).balance == 1 ether, "only vault 3 was swept");
        }
    }
}
