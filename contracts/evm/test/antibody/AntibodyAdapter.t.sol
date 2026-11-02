// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import {AntibodyAdapter, AntibodyProxy} from "../../src/antibody/AntibodyAdapter.sol";
import {AcmeGuard} from "../../src/acme/AcmeGuard.sol";

interface Vm {
    function addr(uint256) external pure returns (address);
    function sign(uint256, bytes32) external pure returns (uint8, bytes32, bytes32);
    function expectRevert(bytes4) external;
    function warp(uint256) external;
    function chainId(uint256) external;
}

/// An implementation behind the proxy: one storage slot, read by `value()`.
contract ImplV1 {
    uint256 public value;

    function init(uint256 v) external {
        value = v;
    }

    function version() external pure returns (uint256) {
        return 1;
    }
}

contract ImplV2 {
    uint256 public value;

    function version() external pure returns (uint256) {
        return 2;
    }
}

/// A target whose `pause()` always reverts: the adapter must skip it and carry on.
contract BrokenTarget {
    function pause() external pure {
        revert("broken");
    }
}

contract AntibodyAdapterTest {
    Vm constant vm = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));
    uint256 constant GUARDIAN_KEY = 0xA11CE;
    uint256 constant OTHER_KEY = 0xB0B;
    bytes32 constant PROGRAM = keccak256("antibody.testnet/0");

    AntibodyAdapter adapter;
    AntibodyProxy proxy;
    AcmeGuard guard;
    BrokenTarget broken;
    ImplV2 v2;

    function setUp() public {
        vm.warp(1_800_000_000);
        guard = new AcmeGuard(address(this));
        broken = new BrokenTarget();
        // The adapter's address is known before deployment (CREATE2 in production; here a CREATE whose
        // nonce is known: AcmeGuard 1, BrokenTarget 2, ImplV1 3, the proxy 4, ImplV2 5, the adapter 6).
        address predicted = _createAddress(6);
        ImplV1 v1 = new ImplV1();
        proxy = new AntibodyProxy(address(v1), predicted, abi.encodeCall(ImplV1.init, (42)));
        v2 = new ImplV2();
        address[] memory targets = new address[](2);
        targets[0] = address(broken);
        targets[1] = address(guard);
        address[] memory proxies = new address[](1);
        proxies[0] = address(proxy);
        adapter = new AntibodyAdapter(vm.addr(GUARDIAN_KEY), PROGRAM, targets, proxies);
        require(address(adapter) == predicted, "predicted adapter address");
        guard.setPauser(address(adapter), true);
    }

    /// The address of this contract's CREATE with `nonce` (RLP of the address and a nonce below 128).
    function _createAddress(uint8 nonce) internal view returns (address) {
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xd6), bytes1(0x94), address(this), nonce)))));
    }

    function _pausePermit(uint32 round, uint64 ticket) internal view returns (AntibodyAdapter.PausePermit memory) {
        return AntibodyAdapter.PausePermit({program: PROGRAM, round: round, ticket: ticket, deadline: block.timestamp + 600});
    }

    function _sign(uint256 key, bytes32 digest) internal pure returns (uint8, bytes32, bytes32) {
        return vm.sign(key, digest);
    }

    function test_pausePermitPausesEveryTargetBestEffort() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.pauseDigest(p));
        // Anyone relays.
        adapter.pause(p, v, r, s);
        require(guard.paused(), "the guard is paused although the first target reverted");
    }

    function test_aPermitIsUsedOnce() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.pauseDigest(p));
        adapter.pause(p, v, r, s);
        vm.expectRevert(AntibodyAdapter.AlreadyUsed.selector);
        adapter.pause(p, v, r, s);
    }

    function test_onlyTheProjectsGuardianSigns() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        (uint8 v, bytes32 r, bytes32 s) = _sign(OTHER_KEY, adapter.pauseDigest(p));
        vm.expectRevert(AntibodyAdapter.BadSignature.selector);
        adapter.pause(p, v, r, s);
    }

    function test_malleableSignaturesAreRefused() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.pauseDigest(p));
        uint256 n = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141;
        vm.expectRevert(AntibodyAdapter.BadSignature.selector);
        adapter.pause(p, v == 27 ? 28 : 27, r, bytes32(n - uint256(s)));
    }

    function test_expiredPermitsAreRefused() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.pauseDigest(p));
        vm.warp(p.deadline + 1);
        vm.expectRevert(AntibodyAdapter.Expired.selector);
        adapter.pause(p, v, r, s);
    }

    function test_aPermitForAnotherProgramIsRefused() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        p.program = keccak256("antibody.testnet/1");
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.pauseDigest(p));
        vm.expectRevert(AntibodyAdapter.WrongProgram.selector);
        adapter.pause(p, v, r, s);
    }

    function test_aSignatureForAnotherChainIsRefused() public {
        AntibodyAdapter.PausePermit memory p = _pausePermit(0, 7);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.pauseDigest(p));
        vm.chainId(84532);
        vm.expectRevert(AntibodyAdapter.BadSignature.selector);
        adapter.pause(p, v, r, s);
    }

    function _releasePermit(bytes32 codeHash) internal view returns (AntibodyAdapter.ReleasePermit memory) {
        return AntibodyAdapter.ReleasePermit({program: PROGRAM, round: 1, proxy: address(proxy), codeHash: codeHash, deadline: block.timestamp + 600});
    }

    function test_releaseUpgradesToTheExactCodeHash() public {
        AntibodyAdapter.ReleasePermit memory p = _releasePermit(address(v2).codehash);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.releaseDigest(p));
        require(ImplV1(address(proxy)).version() == 1 && ImplV1(address(proxy)).value() == 42, "v1 before");
        adapter.release(p, address(v2), v, r, s);
        require(ImplV2(address(proxy)).version() == 2, "v2 after");
        require(ImplV2(address(proxy)).value() == 42, "storage stays in the proxy");
    }

    function test_releaseRefusesAnyOtherCode() public {
        AntibodyAdapter.ReleasePermit memory p = _releasePermit(address(v2).codehash);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.releaseDigest(p));
        ImplV1 other = new ImplV1();
        vm.expectRevert(AntibodyAdapter.CodeHashMismatch.selector);
        adapter.release(p, address(other), v, r, s);
    }

    function test_onlyTheAdapterUpgradesTheProxy() public {
        vm.expectRevert(AntibodyProxy.NotAdmin.selector);
        AntibodyProxy(payable(address(proxy))).upgradeTo(address(v2));
    }

    function test_releaseOnlyTargetsRegisteredProxies() public {
        AntibodyAdapter.ReleasePermit memory p = _releasePermit(address(v2).codehash);
        p.proxy = address(guard);
        (uint8 v, bytes32 r, bytes32 s) = _sign(GUARDIAN_KEY, adapter.releaseDigest(p));
        vm.expectRevert(AntibodyAdapter.NotAProxy.selector);
        adapter.release(p, address(v2), v, r, s);
    }
}
