// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// Rules as code for the "Acme Lend" fixture program. The runtime bytecode of this contract (with
// its immutables) is what the program commits to; Antibody injects it in its fork only.

import {IAntibodyInvariants} from "../antibody/IAntibodyInvariants.sol";
import {IAcmeToken} from "./AcmeTokens.sol";
import {AcmeLend} from "./AcmeLend.sol";
import {AcmeVault} from "./AcmeVault.sol";

contract AcmeInvariants is IAntibodyInvariants {
    /// bit 0 — Critical: lender claims are covered at the committed reference price:
    ///   cash + Σ_i min(debt_i, collateral_i × P_REF × LT) ≥ totalSupplied
    uint256 public constant SOLVENCY = 1 << 0;
    /// bit 1 — High: accrued fees + treasury balance never decrease (collectFees moves, never burns)
    uint256 public constant FEES_TO_TREASURY = 1 << 1;
    /// bit 2 — Medium: the vault share price never falls for the remaining holders
    uint256 public constant SHARE_PRICE = 1 << 2;

    uint256 public constant LT_BPS = 8_000;

    AcmeLend public immutable lend;
    AcmeVault public immutable vault;
    IAcmeToken public immutable acUSD;
    address public immutable treasury;
    /// Committed reference price of acETH in acUSD, 1e18-scaled (not read from any pool).
    uint256 public immutable P_REF;

    constructor(AcmeLend lend_, AcmeVault vault_, IAcmeToken acUSD_, address treasury_, uint256 pRef) {
        lend = lend_;
        vault = vault_;
        acUSD = acUSD_;
        treasury = treasury_;
        P_REF = pRef;
    }

    function snapshot() external view returns (bytes memory) {
        return abi.encode(_feesAndTreasury(), vault.totalAssets(), vault.totalSupply());
    }

    function check(bytes calldata pre) external view returns (uint256 broken) {
        (uint256 fees0, uint256 assets0, uint256 shares0) = abi.decode(pre, (uint256, uint256, uint256));
        if (!solvent()) broken |= SOLVENCY;
        if (_feesAndTreasury() < fees0) broken |= FEES_TO_TREASURY;
        uint256 assets1 = vault.totalAssets();
        uint256 shares1 = vault.totalSupply();
        // assets1/shares1 >= assets0/shares0, compared exactly
        if (shares0 != 0 && shares1 != 0 && assets1 * shares0 < assets0 * shares1) broken |= SHARE_PRICE;
    }

    function solvent() public view returns (bool) {
        uint256 covered = acUSD.balanceOf(address(lend));
        uint256 n = lend.borrowersLength();
        for (uint256 i; i < n; ++i) {
            address b = lend.borrowerAt(i);
            uint256 cap = lend.collateralOf(b) * P_REF / 1e18 * LT_BPS / 10_000;
            uint256 debt = lend.debtOf(b);
            covered += debt < cap ? debt : cap;
        }
        return covered >= lend.totalSupplied();
    }

    function _feesAndTreasury() internal view returns (uint256) {
        return vault.accruedFees() + acUSD.balanceOf(treasury);
    }
}
