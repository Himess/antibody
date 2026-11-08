// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE: never deploy it. A small fork-mode deployment for `antibody window`'s tests
// (crates/cli/src/window.rs): a coin, a vault holding it with a planted bug (anyone can sweep it), a helper that is
// not a target, and the rule that the vault keeps its coins. `scripts/demo/window_fixture.py` builds it with
// Foundry and writes the artifacts the tests read (fixtures/window/artifacts).

interface IRules {
    function snapshot() external view returns (bytes memory);
    function check(bytes calldata pre) external view returns (uint256 broken);
}

contract Coin {
    string public name;
    address public immutable minter;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;

    constructor(string memory name_, address minter_) {
        name = name_;
        minter = minter_;
    }

    function mint(address to, uint256 amount) external {
        require(msg.sender == minter, "coin: not the minter");
        totalSupply += amount;
        balanceOf[to] += amount;
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        return true;
    }
}

contract Vault {
    Coin public immutable coin;
    address public immutable owner;

    constructor(Coin coin_, address owner_) {
        coin = coin_;
        owner = owner_;
    }

    /// The planted bug: no owner check.
    function sweep(address to) external {
        coin.transfer(to, coin.balanceOf(address(this)));
    }
}

/// Not a target: a helper the deployment creates, with a `bytes` constructor argument.
contract Note {
    bytes public data;

    constructor(bytes memory data_) {
        data = data_;
    }
}

contract VaultRules is IRules {
    /// bit 0: the vault never holds fewer coins after a proof than before it.
    uint256 public constant VAULT_KEEPS_ITS_COINS = 1 << 0;

    Vault public immutable vault;
    Coin public immutable coin;

    constructor(Vault vault_, Coin coin_) {
        vault = vault_;
        coin = coin_;
    }

    function snapshot() external view returns (bytes memory) {
        return abi.encode(coin.balanceOf(address(vault)));
    }

    function check(bytes calldata pre) external view returns (uint256 broken) {
        if (coin.balanceOf(address(vault)) < abi.decode(pre, (uint256))) broken |= VAULT_KEEPS_ITS_COINS;
    }
}
