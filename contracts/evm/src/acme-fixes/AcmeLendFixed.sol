// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// The project's fix for planted bug #1, shipped as a round-1 code override (docs/rounds.md): the
// runtime code below replaces AcmeLend's at the same address and the same anchor. The storage layout
// is unchanged (the reference price is an immutable), and the other planted bugs are left as they
// are, so their claims carry over into round 1.

import {AcmeGuard} from "../acme/AcmeGuard.sol";
import {IAcmeToken} from "../acme/AcmeTokens.sol";
import {AcmePool} from "../acme/AcmePool.sol";
import {AcmeLend} from "../acme/AcmeLend.sol";

contract AcmeLendFixed is AcmeLend {
    /// @notice acUSD per acETH (1e18-scaled) the project commits to; collateral is never valued above it.
    uint256 public immutable referencePrice;

    constructor(AcmeGuard guard_, IAcmeToken acUSD_, IAcmeToken acETH_, AcmePool pool_, uint256 referencePrice_)
        AcmeLend(guard_, acUSD_, acETH_, pool_)
    {
        referencePrice = referencePrice_;
    }

    /// @notice The pool's spot price can only lower the valuation: a trade or a donation that pumps
    /// the pool no longer raises anyone's borrow limit.
    function collateralPrice() public view override returns (uint256) {
        uint256 spot = pool.spotPrice();
        return spot < referencePrice ? spot : referencePrice;
    }
}
