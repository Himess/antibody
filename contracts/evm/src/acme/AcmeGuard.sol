// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// "Acme Lend" is a fictional protocol with planted bugs, used only to demonstrate Antibody.

/// @title AcmeGuard — one pause switch for every Acme contract on a chain.
/// @notice The Antibody guardian (an EVM address derived from a NEAR account via Chain Signatures)
/// holds the scoped, revocable PAUSER role. `pause()` is idempotent so duplicate or racing
/// guardian transactions never revert. Only the admin can unpause or change pausers.
contract AcmeGuard {
    address public immutable admin;
    mapping(address => bool) public isPauser;
    bool public paused;

    event PauserSet(address indexed pauser, bool enabled);
    event Paused(address indexed by);
    event Unpaused(address indexed by);

    error NotAdmin();
    error NotPauser();

    constructor(address admin_) {
        admin = admin_;
    }

    function setPauser(address pauser, bool enabled) external {
        if (msg.sender != admin) revert NotAdmin();
        isPauser[pauser] = enabled;
        emit PauserSet(pauser, enabled);
    }

    function pause() external {
        if (!isPauser[msg.sender] && msg.sender != admin) revert NotPauser();
        if (!paused) {
            paused = true;
            emit Paused(msg.sender);
        }
    }

    function unpause() external {
        if (msg.sender != admin) revert NotAdmin();
        paused = false;
        emit Unpaused(msg.sender);
    }
}

/// @notice Base for Acme contracts: all state-changing user functions stop while the guard is paused.
abstract contract GuardPausable {
    AcmeGuard public immutable guard;

    error ProtocolPaused();

    constructor(AcmeGuard guard_) {
        guard = guard_;
    }

    modifier whenNotPaused() {
        if (guard.paused()) revert ProtocolPaused();
        _;
    }

    function paused() external view returns (bool) {
        return guard.paused();
    }
}
