// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// "Acme Lend" is a fictional protocol with planted bugs, used only to demonstrate Antibody.

import {AcmeGuard, GuardPausable} from "./AcmeGuard.sol";
import {IAcmeToken} from "./AcmeTokens.sol";
import {AcmeLend} from "./AcmeLend.sol";

/// @title AcmeVault — lender vault: deposit acUSD, receive shares; the vault supplies AcmeLend.
/// @notice A 0.10% deposit fee accrues to `accruedFees` (held as idle acUSD in the vault) and
/// belongs to the treasury. Shares are non-transferable (fixture simplification). Users reach it
/// through an AntibodyProxy, so a passed release gate can ship a fix; its constructor sets immutables
/// only, and `approveLend` is the proxy's initializer.
contract AcmeVault is GuardPausable {
    uint256 public constant DEPOSIT_FEE_BPS = 10;

    IAcmeToken public immutable acUSD;
    AcmeLend public immutable lend;
    address public immutable treasury;
    address public immutable admin;

    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    uint256 public accruedFees;

    event Deposit(address indexed owner, uint256 assets, uint256 shares, uint256 fee);
    event Withdraw(address indexed owner, uint256 assets, uint256 shares);
    event YieldDonated(address indexed from, uint256 assets);
    event FeesCollected(address indexed to, uint256 amount);

    error TransferFailed();
    error ZeroShares();

    constructor(AcmeGuard guard_, IAcmeToken acUSD_, AcmeLend lend_, address treasury_, address admin_)
        GuardPausable(guard_)
    {
        acUSD = acUSD_;
        lend = lend_;
        treasury = treasury_;
        admin = admin_;
    }

    /// @notice Lets AcmeLend pull the vault's acUSD. Run through the proxy at deployment; repeating it
    /// changes nothing.
    function approveLend() external {
        acUSD.approve(address(lend), type(uint256).max);
    }

    /// @notice Assets backing the shares: what the vault has supplied to AcmeLend.
    function totalAssets() public view returns (uint256) {
        return lend.suppliedBy(address(this));
    }

    function deposit(uint256 assets) external whenNotPaused returns (uint256 shares) {
        return _deposit(msg.sender, msg.sender, assets);
    }

    /// @notice A convenience for routers: deposits acUSD that `from` approved to the vault.
    /// PLANTED BUG #5 (gap lane, user harm): nothing checks that `from` asked for this, and the shares
    /// go to the caller. Anyone can take the funds of any account that approved the vault. The pool
    /// stays solvent, fees and the share price are untouched: no committed invariant breaks.
    function depositFor(address from, uint256 assets) external virtual whenNotPaused returns (uint256 shares) {
        return _deposit(from, msg.sender, assets);
    }

    function _deposit(address from, address to, uint256 assets) internal returns (uint256 shares) {
        if (!acUSD.transferFrom(from, address(this), assets)) revert TransferFailed();
        uint256 fee = assets * DEPOSIT_FEE_BPS / 10_000;
        uint256 net = assets - fee;
        accruedFees += fee;
        uint256 supply = totalSupply;
        shares = supply == 0 ? net : net * supply / totalAssets(); // rounds down: favours the vault
        if (shares == 0) revert ZeroShares();
        lend.supply(net);
        balanceOf[to] += shares;
        totalSupply = supply + shares;
        emit Deposit(to, assets, shares, fee);
    }

    function withdraw(uint256 assets) external whenNotPaused returns (uint256 shares) {
        // PLANTED BUG #4 (Medium — the project's declared known issue): burned shares are rounded
        // DOWN. ERC-4626 requires rounding up, so every withdrawal can leak up to one share's worth
        // from the remaining holders (with shares worth > 1 wei, `withdraw(1)` burns nothing).
        shares = assets * totalSupply / totalAssets();
        balanceOf[msg.sender] -= shares;
        totalSupply -= shares;
        lend.withdrawSupply(assets);
        if (!acUSD.transfer(msg.sender, assets)) revert TransferFailed();
        emit Withdraw(msg.sender, assets, shares);
    }

    /// @notice Lender incentive: anyone may add yield, raising the share price for all holders.
    function donateYield(uint256 assets) external whenNotPaused {
        if (!acUSD.transferFrom(msg.sender, address(this), assets)) revert TransferFailed();
        lend.supply(assets);
        emit YieldDonated(msg.sender, assets);
    }

    /// @notice Moves accrued fees to the treasury. Anyone may call; funds can only go to `treasury`.
    function collectFees() external {
        uint256 amount = accruedFees;
        accruedFees = 0;
        if (!acUSD.transfer(treasury, amount)) revert TransferFailed();
        emit FeesCollected(treasury, amount);
    }

    /// @notice Emergency fee rescue, meant for the admin only.
    /// PLANTED BUG #2 (High): missing access control — anyone can send the fees anywhere.
    function sweepFees(address to) external virtual {
        uint256 amount = accruedFees;
        accruedFees = 0;
        if (!acUSD.transfer(to, amount)) revert TransferFailed();
        emit FeesCollected(to, amount);
    }
}

/// @title AcmeTreasury — receives protocol fees; only the admin can move funds out.
contract AcmeTreasury {
    address public immutable admin;

    constructor(address admin_) {
        admin = admin_;
    }

    function execute(address to, bytes calldata data) external returns (bytes memory) {
        require(msg.sender == admin, "treasury: not admin");
        (bool ok, bytes memory out) = to.call(data);
        require(ok, "treasury: call failed");
        return out;
    }
}
