// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// The project's final fix, shipped as a round-2 code override (docs/rounds.md): AcmeLendFinal replaces
// AcmeLend's implementation (planted bugs #1 and #3) and AcmeVaultFixed replaces AcmeVault's (planted
// bug #2), each at its implementation's address and the same anchor. Storage layouts are unchanged.
// Planted bug #4, the project's declared known issue, is left as it is.

import {AcmeGuard} from "./AcmeGuard.sol";
import {IAcmeToken} from "./AcmeTokens.sol";
import {AcmePool} from "./AcmePool.sol";
import {AcmeLend} from "./AcmeLend.sol";
import {AcmeLendFixed} from "./AcmeLendFixed.sol";
import {AcmeVault} from "./AcmeVault.sol";

/// @notice Round 1's reference price (bug #1), and a repayment that must actually arrive (bug #3).
contract AcmeLendFinal is AcmeLendFixed {
    constructor(AcmeGuard guard_, IAcmeToken acUSD_, IAcmeToken acETH_, AcmePool pool_, uint256 referencePrice_)
        AcmeLendFixed(guard_, acUSD_, acETH_, pool_, referencePrice_)
    {}

    function repay(uint256 amount) external override whenNotPaused {
        if (!acUSD.transferFrom(msg.sender, address(this), amount)) revert TransferFailed();
        debtOf[msg.sender] -= amount;
        totalDebt -= amount;
        emit Repaid(msg.sender, amount);
    }
}

/// @notice The fee rescue is the admin's only (bug #2).
contract AcmeVaultFixed is AcmeVault {
    error NotAdmin();

    constructor(AcmeGuard guard_, IAcmeToken acUSD_, AcmeLend lend_, address treasury_, address admin_)
        AcmeVault(guard_, acUSD_, lend_, treasury_, admin_)
    {}

    function sweepFees(address to) external override {
        if (msg.sender != admin) revert NotAdmin();
        uint256 amount = accruedFees;
        accruedFees = 0;
        if (!acUSD.transfer(to, amount)) revert TransferFailed();
        emit FeesCollected(to, amount);
    }
}
