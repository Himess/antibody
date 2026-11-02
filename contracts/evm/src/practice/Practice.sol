// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// TUTORIAL TARGET — never deploy to mainnet. The practice program's vaults share one textbook bug,
// published with its PoC (docs/practice.md), so anyone can practise the whole loop on testnet: commit,
// seal, submit, get paid. Each vault is its own target, so each practice account is paid for its own.

import {IAntibodyInvariants} from "../antibody/IAntibodyInvariants.sol";

/// @title PracticeVault — holds a little ether for its owner.
contract PracticeVault {
    address public immutable owner;

    constructor(address owner_) payable {
        owner = owner_;
    }

    receive() external payable {}

    /// @notice Meant for the owner.
    /// TUTORIAL BUG (missing access control): anyone can sweep the vault's ether to any address.
    function sweep(address payable to) external {
        to.transfer(address(this).balance);
    }
}

/// @title PracticeFactory — the practice program's vaults, at CREATE2 addresses anyone can compute.
contract PracticeFactory {
    address public immutable owner;
    uint256 public immutable count;

    constructor(uint256 count_) payable {
        owner = msg.sender;
        count = count_;
        uint256 each = msg.value / count_;
        for (uint256 i; i < count_; ++i) {
            new PracticeVault{salt: bytes32(i), value: each}(msg.sender);
        }
    }

    /// @notice The vault at index `i` (a practice account is assigned one).
    function vaultAt(uint256 i) public view returns (address) {
        return vaultOf(address(this), owner, i);
    }

    function vaultOf(address factory, address owner_, uint256 i) public pure returns (address) {
        bytes32 init = keccak256(abi.encodePacked(type(PracticeVault).creationCode, abi.encode(owner_)));
        return address(uint160(uint256(keccak256(abi.encodePacked(bytes1(0xff), factory, bytes32(i), init)))));
    }
}

/// @title PracticeInvariants — the practice program's one rule, as code.
contract PracticeInvariants is IAntibodyInvariants {
    /// bit 0 — Low: no practice vault loses its ether
    uint256 public constant VAULT_KEEPS_ITS_ETHER = 1 << 0;

    PracticeFactory public immutable factory;
    uint256 public immutable count;

    constructor(PracticeFactory factory_) {
        factory = factory_;
        count = factory_.count();
    }

    function snapshot() external view returns (bytes memory) {
        uint256[] memory b = new uint256[](count);
        for (uint256 i; i < count; ++i) {
            b[i] = factory.vaultAt(i).balance;
        }
        return abi.encode(b);
    }

    function check(bytes calldata pre) external view returns (uint256 broken) {
        uint256[] memory b = abi.decode(pre, (uint256[]));
        for (uint256 i; i < count; ++i) {
            if (factory.vaultAt(i).balance < b[i]) broken |= VAULT_KEEPS_ITS_ETHER;
        }
    }
}
