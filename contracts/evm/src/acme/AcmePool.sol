// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// "Acme Lend" is a fictional protocol with planted bugs, used only to demonstrate Antibody.

import {AcmeGuard, GuardPausable} from "./AcmeGuard.sol";
import {IAcmeToken} from "./AcmeTokens.sol";

/// @title AcmePool — a thin constant-product acETH/acUSD pool (0.3% fee).
/// @notice Liquidity is provided by the project only (fixture simplification). The pool itself is
/// correct; using its spot price as a lending oracle is not (see AcmeLend).
contract AcmePool is GuardPausable {
    uint256 public constant FEE_BPS = 30;

    IAcmeToken public immutable acETH;
    IAcmeToken public immutable acUSD;
    address public immutable admin;
    uint256 public reserveETH;
    uint256 public reserveUSD;

    event Swap(address indexed sender, address indexed tokenIn, uint256 amountIn, uint256 amountOut, address indexed to);
    event Sync(uint256 reserveETH, uint256 reserveUSD);

    error NotAdmin();
    error BadToken();
    error Slippage();
    error TransferFailed();

    constructor(AcmeGuard guard_, IAcmeToken acETH_, IAcmeToken acUSD_, address admin_) GuardPausable(guard_) {
        acETH = acETH_;
        acUSD = acUSD_;
        admin = admin_;
    }

    function addLiquidity(uint256 ethAmount, uint256 usdAmount) external {
        if (msg.sender != admin) revert NotAdmin();
        _pull(acETH, ethAmount);
        _pull(acUSD, usdAmount);
        _sync();
    }

    function swap(address tokenIn, uint256 amountIn, uint256 minOut, address to)
        external
        whenNotPaused
        returns (uint256 amountOut)
    {
        bool ethIn = tokenIn == address(acETH);
        if (!ethIn && tokenIn != address(acUSD)) revert BadToken();
        (uint256 rIn, uint256 rOut) = ethIn ? (reserveETH, reserveUSD) : (reserveUSD, reserveETH);
        _pull(IAcmeToken(tokenIn), amountIn);
        uint256 inAfterFee = amountIn * (10_000 - FEE_BPS) / 10_000;
        amountOut = rOut * inAfterFee / (rIn + inAfterFee);
        if (amountOut == 0 || amountOut < minOut) revert Slippage();
        if (!(ethIn ? acUSD : acETH).transfer(to, amountOut)) revert TransferFailed();
        _sync();
        emit Swap(msg.sender, tokenIn, amountIn, amountOut, to);
    }

    /// @notice Sets reserves to balances (anyone may call; mirrors Uniswap V2 `sync`).
    function sync() external whenNotPaused {
        _sync();
    }

    /// @notice acUSD per acETH, 1e18-scaled. Instantaneous — moved by any trade or donation.
    function spotPrice() external view returns (uint256) {
        return reserveUSD * 1e18 / reserveETH;
    }

    function _pull(IAcmeToken token, uint256 amount) internal {
        if (!token.transferFrom(msg.sender, address(this), amount)) revert TransferFailed();
    }

    function _sync() internal {
        reserveETH = acETH.balanceOf(address(this));
        reserveUSD = acUSD.balanceOf(address(this));
        emit Sync(reserveETH, reserveUSD);
    }
}
