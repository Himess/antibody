// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @notice A proxy whose implementation only its admin can replace (ERC-1967 slots).
interface IAntibodyUpgradeable {
    function upgradeTo(address implementation) external;
}

/// @title AntibodyAdapter: carries out a program's Antibody permits on one chain.
/// @notice near-antibody decides, under the program's committed policy, when a permit is due; the
/// project's guardian key (a Chain Signatures key derived under the project's own path) signs it as
/// EIP-712; anyone may relay it here. Deployed with CREATE2, so it has the same address on every chain.
/// - `pause` calls `pause()` on each registered target. The calldata is fixed. A target that reverts is
///   skipped and reported (best effort per target).
/// - `release` upgrades a registered proxy to `implementation`, only if its runtime code hash is the one
///   the permit names.
/// Each permit names a replay key (a pause: its round and ticket; a release: its round and proxy), used
/// once, and an expiry. Nothing here holds funds or can do anything else.
contract AntibodyAdapter {
    struct PausePermit {
        bytes32 program;
        uint32 round;
        uint64 ticket;
        uint256 deadline;
    }

    struct ReleasePermit {
        bytes32 program;
        uint32 round;
        address proxy;
        bytes32 codeHash;
        uint256 deadline;
    }

    bytes32 private constant DOMAIN_TYPEHASH =
        keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
    bytes32 public constant PAUSE_TYPEHASH = keccak256("PausePermit(bytes32 program,uint32 round,uint64 ticket,uint256 deadline)");
    bytes32 public constant RELEASE_TYPEHASH =
        keccak256("ReleasePermit(bytes32 program,uint32 round,address proxy,bytes32 codeHash,uint256 deadline)");
    /// secp256k1n / 2: signatures with a larger `s` are malleable twins and are refused.
    uint256 private constant HALF_N = 0x7fffffffffffffffffffffffffffffff5d576e7357a4501ddfe92f46681b20a0;

    /// The project's guardian address (its Chain Signatures key).
    address public immutable guardian;
    /// keccak256 of the program key (`<near-antibody account>/<id>`).
    bytes32 public immutable program;
    address[] private targets;
    mapping(address => bool) public isProxy;
    mapping(bytes32 => bool) public used;

    event PauseCalled(bytes32 indexed program, uint32 round, uint64 ticket, address indexed target, bool ok);
    event Released(bytes32 indexed program, uint32 round, address indexed proxy, address implementation, bytes32 codeHash);

    error WrongProgram();
    error Expired();
    error AlreadyUsed();
    error BadSignature();
    error NotAProxy();
    error CodeHashMismatch();

    constructor(address guardian_, bytes32 program_, address[] memory targets_, address[] memory proxies) {
        guardian = guardian_;
        program = program_;
        targets = targets_;
        for (uint256 i = 0; i < proxies.length; i++) {
            isProxy[proxies[i]] = true;
        }
    }

    function pauseTargets() external view returns (address[] memory) {
        return targets;
    }

    function domainSeparator() public view returns (bytes32) {
        return keccak256(abi.encode(DOMAIN_TYPEHASH, keccak256("Antibody"), keccak256("1"), block.chainid, address(this)));
    }

    function pauseDigest(PausePermit calldata p) public view returns (bytes32) {
        bytes32 h = keccak256(abi.encode(PAUSE_TYPEHASH, p.program, p.round, p.ticket, p.deadline));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), h));
    }

    function releaseDigest(ReleasePermit calldata p) public view returns (bytes32) {
        bytes32 h = keccak256(abi.encode(RELEASE_TYPEHASH, p.program, p.round, p.proxy, p.codeHash, p.deadline));
        return keccak256(abi.encodePacked("\x19\x01", domainSeparator(), h));
    }

    /// Calls `pause()` on every target. Idempotent for the targets; the permit itself is used once.
    function pause(PausePermit calldata p, uint8 v, bytes32 r, bytes32 s) external {
        _check(p.program, p.deadline, pauseDigest(p), v, r, s);
        _use(keccak256(abi.encode("pause", p.round, p.ticket)));
        for (uint256 i = 0; i < targets.length; i++) {
            (bool ok,) = targets[i].call(abi.encodeWithSelector(0x8456cb59));
            emit PauseCalled(p.program, p.round, p.ticket, targets[i], ok);
        }
    }

    /// Upgrades `p.proxy` to `implementation`, whose runtime code must hash to `p.codeHash`.
    function release(ReleasePermit calldata p, address implementation, uint8 v, bytes32 r, bytes32 s) external {
        _check(p.program, p.deadline, releaseDigest(p), v, r, s);
        if (!isProxy[p.proxy]) revert NotAProxy();
        if (implementation.codehash != p.codeHash) revert CodeHashMismatch();
        _use(keccak256(abi.encode("release", p.round, p.proxy)));
        IAntibodyUpgradeable(p.proxy).upgradeTo(implementation);
        emit Released(p.program, p.round, p.proxy, implementation, p.codeHash);
    }

    function _check(bytes32 program_, uint256 deadline, bytes32 digest, uint8 v, bytes32 r, bytes32 s) private view {
        if (program_ != program) revert WrongProgram();
        if (block.timestamp > deadline) revert Expired();
        if (uint256(s) > HALF_N || (v != 27 && v != 28)) revert BadSignature();
        address who = ecrecover(digest, v, r, s);
        if (who == address(0) || who != guardian) revert BadSignature();
    }

    function _use(bytes32 key) private {
        if (used[key]) revert AlreadyUsed();
        used[key] = true;
    }
}

/// @title AntibodyProxy: an ERC-1967 proxy whose implementation only the program's adapter replaces.
/// @notice A release permit names the new implementation's code hash; nobody else can upgrade. The
/// deployer is admin until it hands the proxy to the adapter, which exists only once the program does
/// (`changeAdmin`, once the adapter is deployed).
contract AntibodyProxy {
    /// bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)
    bytes32 private constant IMPLEMENTATION_SLOT = 0x360894a13ba1a3210667c828492db98dca3e2076cc3735a920a3ca505d382bbc;
    /// bytes32(uint256(keccak256("eip1967.proxy.admin")) - 1)
    bytes32 private constant ADMIN_SLOT = 0xb53127684a568b3173ae13b9f8a6016e243e63b6e8ee1178d6a717850b5d6103;

    event Upgraded(address indexed implementation);
    event AdminChanged(address previousAdmin, address newAdmin);

    error NotAdmin();

    constructor(address implementation, address admin, bytes memory init) {
        assembly {
            sstore(IMPLEMENTATION_SLOT, implementation)
            sstore(ADMIN_SLOT, admin)
        }
        emit Upgraded(implementation);
        if (init.length > 0) {
            (bool ok, bytes memory err) = implementation.delegatecall(init);
            if (!ok) {
                assembly {
                    revert(add(err, 32), mload(err))
                }
            }
        }
    }

    function _admin() private view returns (address a) {
        assembly {
            a := sload(ADMIN_SLOT)
        }
    }

    /// The admin hands the proxy to a new admin (the program's adapter).
    function changeAdmin(address newAdmin) external {
        address admin = _admin();
        if (msg.sender != admin) revert NotAdmin();
        assembly {
            sstore(ADMIN_SLOT, newAdmin)
        }
        emit AdminChanged(admin, newAdmin);
    }

    function upgradeTo(address implementation) external {
        if (msg.sender != _admin()) revert NotAdmin();
        assembly {
            sstore(IMPLEMENTATION_SLOT, implementation)
        }
        emit Upgraded(implementation);
    }

    fallback() external payable {
        assembly {
            let impl := sload(IMPLEMENTATION_SLOT)
            calldatacopy(0, 0, calldatasize())
            let ok := delegatecall(gas(), impl, 0, calldatasize(), 0, 0)
            returndatacopy(0, 0, returndatasize())
            switch ok
            case 0 { revert(0, returndatasize()) }
            default { return(0, returndatasize()) }
        }
    }

    receive() external payable {}
}
