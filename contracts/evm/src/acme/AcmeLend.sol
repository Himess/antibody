// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// "Acme Lend" is a fictional protocol with planted bugs, used only to demonstrate Antibody.

import {AcmeGuard, GuardPausable} from "./AcmeGuard.sol";
import {IAcmeToken} from "./AcmeTokens.sol";
import {AcmePool} from "./AcmePool.sol";

/// @title AcmeLend — lending market: post acETH collateral, borrow acUSD.
/// @notice Lenders (in practice AcmeVault) supply acUSD; the market's lender claims are
/// `totalSupplied`. No interest (fixture simplification), so with healthy loans
/// `cash + Σ debt == totalSupplied` holds exactly.
contract AcmeLend is GuardPausable {
    uint256 public constant LTV_BPS = 7_500;

    IAcmeToken public immutable acUSD;
    IAcmeToken public immutable acETH;
    AcmePool public immutable pool;

    uint256 public totalSupplied;
    uint256 public totalDebt;
    mapping(address => uint256) public suppliedBy;
    mapping(address => uint256) public collateralOf;
    mapping(address => uint256) public debtOf;
    address[] internal _borrowers;
    mapping(address => bool) public isBorrower;

    event Supplied(address indexed lender, uint256 amount);
    event SupplyWithdrawn(address indexed lender, uint256 amount);
    event CollateralDeposited(address indexed user, uint256 amount);
    event CollateralWithdrawn(address indexed user, uint256 amount);
    event Borrowed(address indexed user, uint256 amount);
    event Repaid(address indexed user, uint256 amount);

    error Unhealthy();
    error TransferFailed();

    constructor(AcmeGuard guard_, IAcmeToken acUSD_, IAcmeToken acETH_, AcmePool pool_) GuardPausable(guard_) {
        acUSD = acUSD_;
        acETH = acETH_;
        pool = pool_;
    }

    // ---------------------------------------------------------------- lenders

    function supply(uint256 amount) external whenNotPaused {
        if (!acUSD.transferFrom(msg.sender, address(this), amount)) revert TransferFailed();
        suppliedBy[msg.sender] += amount;
        totalSupplied += amount;
        emit Supplied(msg.sender, amount);
    }

    function withdrawSupply(uint256 amount) external whenNotPaused {
        suppliedBy[msg.sender] -= amount;
        totalSupplied -= amount;
        if (!acUSD.transfer(msg.sender, amount)) revert TransferFailed();
        emit SupplyWithdrawn(msg.sender, amount);
    }

    // -------------------------------------------------------------- borrowers

    function depositCollateral(uint256 amount) external whenNotPaused {
        if (!acETH.transferFrom(msg.sender, address(this), amount)) revert TransferFailed();
        collateralOf[msg.sender] += amount;
        _track(msg.sender);
        emit CollateralDeposited(msg.sender, amount);
    }

    function withdrawCollateral(uint256 amount) external whenNotPaused {
        collateralOf[msg.sender] -= amount;
        if (!_healthy(msg.sender)) revert Unhealthy();
        if (!acETH.transfer(msg.sender, amount)) revert TransferFailed();
        emit CollateralWithdrawn(msg.sender, amount);
    }

    function borrow(uint256 amount) external whenNotPaused {
        debtOf[msg.sender] += amount;
        totalDebt += amount;
        _track(msg.sender);
        if (!_healthy(msg.sender)) revert Unhealthy();
        if (!acUSD.transfer(msg.sender, amount)) revert TransferFailed();
        emit Borrowed(msg.sender, amount);
    }

    function repay(uint256 amount) external whenNotPaused {
        // PLANTED BUG #3 (Critical): the return value of transferFrom is ignored. acUSD returns
        // false instead of reverting, so a payment that never happened still clears the debt.
        acUSD.transferFrom(msg.sender, address(this), amount);
        debtOf[msg.sender] -= amount;
        totalDebt -= amount;
        emit Repaid(msg.sender, amount);
    }

    // ------------------------------------------------------------------ views

    /// @notice acUSD per acETH, 1e18-scaled.
    /// PLANTED BUG #1 (Critical): collateral is valued at AcmePool's instantaneous spot price,
    /// which any trade or donation in the thin pool moves.
    function collateralPrice() public view virtual returns (uint256) {
        return pool.spotPrice();
    }

    function borrowersLength() external view returns (uint256) {
        return _borrowers.length;
    }

    function borrowerAt(uint256 i) external view returns (address) {
        return _borrowers[i];
    }

    function _healthy(address user) internal view returns (bool) {
        return collateralOf[user] * collateralPrice() / 1e18 * LTV_BPS / 10_000 >= debtOf[user];
    }

    function _track(address user) internal {
        if (!isBorrower[user]) {
            isBorrower[user] = true;
            _borrowers.push(user);
        }
    }
}
