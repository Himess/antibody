// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

/// @title IAntibodyInvariants
/// @notice A program's rules as code. The Antibody verifier injects the runtime bytecode of an
/// implementation at a reserved address in its fork (it is never deployed on-chain), then:
///   1. `pre = snapshot()` at the anchor state, and requires `check(pre) == 0` there;
///   2. runs the hunter's PoC (fresh salted senders, bounded endowment);
///   3. evaluates `check(pre)` at the end of every protocol-entry call and after the PoC.
/// Bit i of `broken` set means invariant i is broken. Both functions must be pure views over
/// protocol state: the injected code has no storage and no constructor runs.
interface IAntibodyInvariants {
    /// @return Encoded values of the anchor state that `check` compares against.
    function snapshot() external view returns (bytes memory);

    /// @param pre The value returned by `snapshot()` at the anchor state.
    /// @return broken Bit i = invariant i is broken.
    function check(bytes calldata pre) external view returns (uint256 broken);
}
