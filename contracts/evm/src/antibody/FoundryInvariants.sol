// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

import {IAntibodyInvariants} from "./IAntibodyInvariants.sol";

/// @title FoundryInvariants
/// @notice Foundry-style invariant functions as a program's rules (docs/foundry.md). Each `invariant_*()`
/// view function the inheriting contract lists in `_invariants()` is one slot, in that order, and a
/// function that reverts is a broken rule. Foundry invariants read the current state, so the snapshot is
/// empty. `antibody invariants from-foundry` writes the inheriting contract from a compiled one. As for
/// any program's rules, the code runs with no storage: the functions may read immutables and the
/// protocol's state, never their own storage.
abstract contract FoundryInvariants is IAntibodyInvariants {
    /// @return The invariant functions' selectors, in slot order (at most 256).
    function _invariants() internal pure virtual returns (bytes4[] memory);

    function snapshot() external pure returns (bytes memory) {
        return "";
    }

    function check(bytes calldata) external view returns (uint256 broken) {
        bytes4[] memory fns = _invariants();
        for (uint256 i; i < fns.length && i < 256; ++i) {
            (bool ok,) = address(this).staticcall(abi.encodeWithSelector(fns[i]));
            if (!ok) broken |= 1 << i;
        }
    }
}
