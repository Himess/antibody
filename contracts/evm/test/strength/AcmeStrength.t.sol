// SPDX-License-Identifier: MIT
pragma solidity 0.8.28;

// DELIBERATELY VULNERABLE TEST FIXTURE — never deploy to mainnet.
// Rule strength (docs/rule-strength.md): the program's rules (AcmeInvariantsV2) under Foundry's
// invariant fuzzer, at the round-2 code (the final fix). `antibody invariants strength` mutates the
// protocol's sources and counts the mutants whose broken behaviour this test catches. The handler drives
// every entry point as an arbitrary actor with bounded amounts. It never triggers the project's declared
// known issue (withdrawals round the burned shares down): its withdrawals are exact, so no share rounds.

import {AcmeSetup} from "../../src/acme/AcmeSetup.sol";
import {AcmeInvariantsV2} from "../../src/acme/AcmeInvariants.sol";
import {IAcmeToken} from "../../src/acme/AcmeTokens.sol";
import {AcmeLendFinal, AcmeVaultFixed} from "../../src/acme-fixes/AcmeFinalFix.sol";

interface Vm {
    function prank(address) external;
    function deal(address, uint256) external;
    function etch(address, bytes calldata) external;
}

Vm constant VM = Vm(address(uint160(uint256(keccak256("hevm cheat code")))));

/// @notice The fuzzer's entry points: each calls one protocol function as one of three actors.
contract AcmeHandler {
    AcmeSetup.Deployment internal d;
    address[3] internal actors = [address(0x1001), address(0x1002), address(0x1003)];

    constructor(AcmeSetup.Deployment memory d_) {
        d = d_;
    }

    function _actor(uint256 s) internal view returns (address) {
        return actors[s % 3];
    }

    function _bound(uint256 x, uint256 lo, uint256 hi) internal pure returns (uint256) {
        return lo + x % (hi - lo + 1);
    }

    /// Usually within the actors' balances; one time in five far beyond them, so a transfer fails.
    function _amount(uint256 x, uint256 lo, uint256 hi) internal pure returns (uint256) {
        return x % 5 == 0 ? _bound(x, 1e24, 1e25) : _bound(x, lo, hi);
    }

    function _gcd(uint256 a, uint256 b) internal pure returns (uint256) {
        while (b != 0) (a, b) = (b, a % b);
        return a;
    }

    // ---- AcmeLend

    function supply(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.lend.supply(_amount(amount, 1e18, 100_000e18));
    }

    function withdrawSupply(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.lend.withdrawSupply(_bound(amount, 1e18, 100_000e18));
    }

    function depositCollateral(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.lend.depositCollateral(_amount(amount, 1e17, 50e18));
    }

    function withdrawCollateral(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.lend.withdrawCollateral(_bound(amount, 1e17, 50e18));
    }

    function borrow(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.lend.borrow(_bound(amount, 1e18, 100_000e18));
    }

    /// Up to the most the protocol itself would lend against the actor's collateral, at its own price.
    function borrowToTheLimit(uint256 s, uint256 pct) external {
        address a = _actor(s);
        uint256 limit = d.lend.collateralOf(a) * d.lend.collateralPrice() / 1e18 * 8_000 / 10_000;
        uint256 debt = d.lend.debtOf(a);
        if (limit <= debt) return;
        VM.prank(a);
        d.lend.borrow((limit - debt) * _bound(pct, 50, 101) / 100);
    }

    function withdrawCollateralShare(uint256 s, uint256 pct) external {
        address a = _actor(s);
        uint256 amount = d.lend.collateralOf(a) * _bound(pct, 1, 100) / 100;
        if (amount == 0) return;
        VM.prank(a);
        d.lend.withdrawCollateral(amount);
    }

    function repay(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.lend.repay(_amount(amount, 1e18, 100_000e18));
    }

    // ---- AcmeVault

    function deposit(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.vault.deposit(_amount(amount, 1e18, 100_000e18));
    }

    /// @dev `from` may be another actor or the protected saver: only `from` itself may deposit its funds.
    function depositFor(uint256 s, uint256 f, uint256 amount) external {
        address from = f % 4 == 3 ? address(d.saver) : _actor(f);
        VM.prank(_actor(s));
        d.vault.depositFor(from, _amount(amount, 1e18, 5_000e18));
    }

    /// @dev An exact withdrawal: `assets * totalSupply` divides by `totalAssets`, so no share rounds
    /// (the declared known issue is the rounding).
    function withdraw(uint256 s, uint256 k) external {
        uint256 ta = d.vault.totalAssets();
        uint256 ts = d.vault.totalSupply();
        if (ta == 0 || ts == 0) return;
        uint256 unit = ta / _gcd(ta, ts);
        uint256 most = 100_000e18 / unit;
        if (most == 0) return;
        VM.prank(_actor(s));
        d.vault.withdraw(_bound(k, 1, most) * unit);
    }

    function donateYield(uint256 s, uint256 amount) external {
        VM.prank(_actor(s));
        d.vault.donateYield(_amount(amount, 1e18, 10_000e18));
    }

    function collectFees(uint256 s) external {
        VM.prank(_actor(s));
        d.vault.collectFees();
    }

    function sweepFees(uint256 s, uint256 t) external {
        VM.prank(_actor(s));
        d.vault.sweepFees(_actor(t));
    }

    function treasuryTransfer(uint256 s) external {
        address a = _actor(s);
        VM.prank(a);
        d.treasury.execute(address(d.usd), abi.encodeCall(IAcmeToken.transfer, (a, d.usd.balanceOf(address(d.treasury)))));
    }

    // ---- AcmePool

    function swap(uint256 s, bool ethIn, uint256 amount) external {
        address a = _actor(s);
        VM.prank(a);
        d.pool.swap(ethIn ? address(d.eth) : address(d.usd), _bound(amount, 1e15, ethIn ? 40e18 : 100_000e18), 0, a);
    }

    function sync(uint256 s) external {
        VM.prank(_actor(s));
        d.pool.sync();
    }

    function addLiquidity(uint256 s, uint256 e, uint256 u) external {
        VM.prank(_actor(s));
        d.pool.addLiquidity(_bound(e, 1e17, 10e18), _bound(u, 1e18, 20_000e18));
    }
}

/// @notice The program's rules hold over every sequence the handler can produce, at the final fix.
contract AcmeStrengthTest {
    AcmeSetup.Deployment internal d;
    AcmeInvariantsV2 internal inv;
    AcmeHandler internal handler;
    bytes internal pre;

    function setUp() public {
        d = AcmeSetup.deploy(address(this));
        AcmeSetup.seed(d, address(this));
        // The round-2 code overrides (as the round's manifest commits them).
        VM.etch(address(d.lendImpl), address(new AcmeLendFinal(d.guard, IAcmeToken(address(d.usd)), IAcmeToken(address(d.eth)), d.pool, AcmeSetup.P_REF)).code);
        VM.etch(address(d.vaultImpl), address(new AcmeVaultFixed(d.guard, IAcmeToken(address(d.usd)), d.lend, address(d.treasury), address(this))).code);
        inv = new AcmeInvariantsV2(d.lend, d.vault, IAcmeToken(address(d.usd)), address(d.treasury), AcmeSetup.P_REF, address(d.saver));
        pre = inv.snapshot();
        handler = new AcmeHandler(d);
        address[3] memory actors = [address(0x1001), address(0x1002), address(0x1003)];
        for (uint256 i; i < 3; ++i) {
            d.usd.mint(actors[i], 200_000e18);
            d.eth.mint(actors[i], 100e18);
            VM.prank(actors[i]);
            d.usd.approve(address(d.lend), type(uint256).max);
            VM.prank(actors[i]);
            d.usd.approve(address(d.vault), type(uint256).max);
            VM.prank(actors[i]);
            d.usd.approve(address(d.pool), type(uint256).max);
            VM.prank(actors[i]);
            d.eth.approve(address(d.lend), type(uint256).max);
            VM.prank(actors[i]);
            d.eth.approve(address(d.pool), type(uint256).max);
        }
    }

    /// Foundry's invariant fuzzer calls only the handler.
    function targetContracts() public view returns (address[] memory t) {
        t = new address[](1);
        t[0] = address(handler);
    }

    function invariant_programRulesHold() public view {
        require(inv.check(pre) == 0, "a program rule breaks");
    }
}
