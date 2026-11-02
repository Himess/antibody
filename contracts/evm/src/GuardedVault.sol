// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @title GuardedVault (Antibody guardian spike target)
/// @notice Minimal protected contract. The Antibody guardian (an address derived via NEAR Chain
/// Signatures) holds a scoped, revocable, pause-only role. Only the admin can unpause or revoke.
/// `pause()` is idempotent so duplicate or racing guardian transactions never revert.
contract GuardedVault {
    address public immutable admin;
    mapping(address => bool) public isPauser;
    bool public paused;
    uint256 public pauseCount;
    mapping(address => uint256) public balanceOf;

    event PauserSet(address indexed pauser, bool enabled);
    event Paused(address indexed by);
    event Unpaused(address indexed by);
    event Deposited(address indexed who, uint256 amount);
    event Withdrawn(address indexed who, uint256 amount);

    error NotAdmin();
    error NotPauser();
    error IsPaused();

    constructor(address pauser) {
        admin = msg.sender;
        isPauser[pauser] = true;
        emit PauserSet(pauser, true);
    }

    modifier onlyAdmin() {
        if (msg.sender != admin) revert NotAdmin();
        _;
    }

    modifier whenNotPaused() {
        if (paused) revert IsPaused();
        _;
    }

    function setPauser(address pauser, bool enabled) external onlyAdmin {
        isPauser[pauser] = enabled;
        emit PauserSet(pauser, enabled);
    }

    /// @notice Pause-only power. Idempotent: pausing an already paused vault is a no-op.
    function pause() external {
        if (!isPauser[msg.sender]) revert NotPauser();
        if (!paused) {
            paused = true;
            pauseCount += 1;
            emit Paused(msg.sender);
        }
    }

    function unpause() external onlyAdmin {
        paused = false;
        emit Unpaused(msg.sender);
    }

    function deposit() external payable whenNotPaused {
        balanceOf[msg.sender] += msg.value;
        emit Deposited(msg.sender, msg.value);
    }

    function withdraw(uint256 amount) external whenNotPaused {
        balanceOf[msg.sender] -= amount;
        emit Withdrawn(msg.sender, amount);
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok, "transfer failed");
    }
}
