// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// "Acme Lend" is a fictional protocol with planted bugs, used only to demonstrate Antibody.
// acUSD / acETH are testnet mocks with no value; the names avoid any real brand.

import {AcmeGuard, GuardPausable} from "./AcmeGuard.sol";

interface IAcmeToken {
    function balanceOf(address) external view returns (uint256);
    function allowance(address, address) external view returns (uint256);
    function approve(address, uint256) external returns (bool);
    function transfer(address, uint256) external returns (bool);
    function transferFrom(address, address, uint256) external returns (bool);
}

abstract contract AcmeERC20 is GuardPausable {
    uint8 public constant decimals = 18;
    address public immutable minter;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    error NotMinter();

    constructor(AcmeGuard guard_, address minter_) GuardPausable(guard_) {
        minter = minter_;
    }

    /// @notice Testnet mock: the project mints supply to seed the protocol.
    function mint(address to, uint256 amount) external {
        if (msg.sender != minter) revert NotMinter();
        _mint(to, amount);
    }

    function approve(address spender, uint256 value) external returns (bool) {
        allowance[msg.sender][spender] = value;
        emit Approval(msg.sender, spender, value);
        return true;
    }

    function _mint(address to, uint256 amount) internal {
        totalSupply += amount;
        balanceOf[to] += amount;
        emit Transfer(address(0), to, amount);
    }

    function _burn(address from, uint256 amount) internal {
        balanceOf[from] -= amount;
        totalSupply -= amount;
        emit Transfer(from, address(0), amount);
    }

    function _move(address from, address to, uint256 amount) internal {
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
    }
}

/// @title acUSD — fixture stablecoin (debt asset).
/// @notice ERC-20 allows returning `false` instead of reverting: this token does exactly that on a
/// failed transfer or transferFrom. Callers must check the return value.
contract AcmeUSD is AcmeERC20 {
    string public constant name = "Acme USD (testnet fixture)";
    string public constant symbol = "acUSD";

    constructor(AcmeGuard guard_, address minter_) AcmeERC20(guard_, minter_) {}

    function transfer(address to, uint256 value) external whenNotPaused returns (bool) {
        if (balanceOf[msg.sender] < value) return false;
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external whenNotPaused returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        if (allowed < value || balanceOf[from] < value) return false;
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - value;
        _move(from, to, value);
        return true;
    }
}

/// @title acETH — fixture wrapped ether (collateral asset).
/// @notice Anyone can wrap ETH 1:1; the project may also mint unbacked supply to seed the
/// protocol (testnet mock). Transfers revert on failure.
contract AcmeETH is AcmeERC20 {
    string public constant name = "Acme Wrapped Ether (testnet fixture)";
    string public constant symbol = "acETH";

    constructor(AcmeGuard guard_, address minter_) AcmeERC20(guard_, minter_) {}

    function deposit() external payable whenNotPaused {
        _mint(msg.sender, msg.value);
    }

    function withdraw(uint256 amount) external whenNotPaused {
        _burn(msg.sender, amount);
        (bool ok,) = msg.sender.call{value: amount}("");
        require(ok, "acETH: ETH transfer failed");
    }

    function transfer(address to, uint256 value) external whenNotPaused returns (bool) {
        _move(msg.sender, to, value);
        return true;
    }

    function transferFrom(address from, address to, uint256 value) external whenNotPaused returns (bool) {
        uint256 allowed = allowance[from][msg.sender];
        require(allowed >= value, "acETH: allowance");
        if (allowed != type(uint256).max) allowance[from][msg.sender] = allowed - value;
        _move(from, to, value);
        return true;
    }
}
