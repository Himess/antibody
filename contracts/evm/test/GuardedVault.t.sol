// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import {GuardedVault} from "../src/GuardedVault.sol";

interface Vm {
    function prank(address) external;
    function expectRevert(bytes4) external;
    function deal(address, uint256) external;
}

contract GuardedVaultTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    GuardedVault v;
    address guardian = address(0xA11CE);
    address user = address(0xB0B);

    function setUp() public {
        v = new GuardedVault(guardian);
        vm.deal(user, 10 ether);
    }

    function test_guardianPausesAndBlocksWithdrawals() public {
        vm.prank(user);
        v.deposit{value: 1 ether}();
        vm.prank(guardian);
        v.pause();
        require(v.paused() && v.pauseCount() == 1, "paused");
        vm.prank(user);
        vm.expectRevert(GuardedVault.IsPaused.selector);
        v.withdraw(1 ether);
    }

    function test_pauseIsIdempotent() public {
        vm.prank(guardian);
        v.pause();
        vm.prank(guardian);
        v.pause(); // duplicate must not revert
        require(v.pauseCount() == 1, "idempotent");
    }

    function test_onlyPauserCanPause_onlyAdminCanUnpauseOrRevoke() public {
        vm.prank(user);
        vm.expectRevert(GuardedVault.NotPauser.selector);
        v.pause();
        vm.prank(guardian);
        v.pause();
        vm.prank(guardian);
        vm.expectRevert(GuardedVault.NotAdmin.selector);
        v.unpause();
        v.unpause();
        v.setPauser(guardian, false);
        vm.prank(guardian);
        vm.expectRevert(GuardedVault.NotPauser.selector);
        v.pause();
    }
}
